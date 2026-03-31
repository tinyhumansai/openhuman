import { useCallback, useEffect, useMemo, useState } from 'react';

import {
  isTauri,
  openhumanAgentServerStatus,
  openhumanServiceInstall,
  openhumanServiceStart,
  openhumanServiceStatus,
  openhumanServiceStop,
  openhumanServiceUninstall,
  type ServiceState,
} from '../../utils/tauriCommands';

interface ServiceBlockingGateProps {
  children: React.ReactNode;
}

type GateStatus = 'checking' | 'ready' | 'blocked';
const SERVICE_GATE_POLL_MS = 3000;
type RefreshOptions = { showChecking?: boolean; clearError?: boolean };

const normalizeServiceState = (state: ServiceState | undefined): string => {
  if (!state) return 'Unknown';
  if (typeof state === 'string') return state;
  if ('Running' in state) return 'Running';
  if ('Stopped' in state) return 'Stopped';
  if ('NotInstalled' in state) return 'NotInstalled';
  if ('Unknown' in state) return `Unknown(${state.Unknown})`;
  return 'Unknown';
};

const ServiceBlockingGate = ({ children }: ServiceBlockingGateProps) => {
  const [gateStatus, setGateStatus] = useState<GateStatus>('checking');
  const [serviceStateText, setServiceStateText] = useState('Unknown');
  const [agentRunning, setAgentRunning] = useState(false);
  const [isOperating, setIsOperating] = useState(false);
  const [operatingLabel, setOperatingLabel] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refreshStatus = useCallback(async (options: RefreshOptions = {}) => {
    const { showChecking = false, clearError = false } = options;
    if (!isTauri()) {
      console.info('[ServiceBlockingGate] Non-Tauri environment detected; gate is ready');
      setGateStatus('ready');
      return;
    }

    if (clearError) {
      setError(null);
    }
    if (showChecking) {
      setGateStatus('checking');
    }
    console.info('[ServiceBlockingGate] Refreshing service + agent status');

    try {
      const [serviceResult, agentResult] = await Promise.allSettled([
        openhumanServiceStatus(),
        openhumanAgentServerStatus(),
      ]);
      const normalized =
        serviceResult.status === 'fulfilled'
          ? normalizeServiceState(serviceResult.value?.result?.state)
          : 'Unknown';
      const serviceRunning = normalized === 'Running';
      const agentIsRunning =
        agentResult.status === 'fulfilled' ? !!agentResult.value?.result?.running : false;
      const gateReady = serviceRunning || agentIsRunning;

      if (serviceResult.status !== 'fulfilled' && !agentIsRunning) {
        throw serviceResult.reason;
      }

      setServiceStateText(prev => (prev === normalized ? prev : normalized));
      setAgentRunning(prev => (prev === agentIsRunning ? prev : agentIsRunning));
      setGateStatus(prev => {
        const next = gateReady ? 'ready' : 'blocked';
        return prev === next ? prev : next;
      });
      setError(prev => (prev ? null : prev));
      console.info('[ServiceBlockingGate] Status refreshed', {
        serviceState: normalized,
        agentRunning: agentIsRunning,
        nextGateStatus: gateReady ? 'ready' : 'blocked',
        passMode: serviceRunning ? 'hard(service)' : agentIsRunning ? 'soft(agent)' : 'blocked',
      });
    } catch (err) {
      setServiceStateText('Unknown');
      setAgentRunning(false);
      setGateStatus('blocked');
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
      console.error('[ServiceBlockingGate] Failed to refresh status', { error: message });
    }
  }, []);

  useEffect(() => {
    void refreshStatus({ showChecking: true });
  }, [refreshStatus]);

  useEffect(() => {
    if (!isTauri()) {
      return;
    }

    console.info('[ServiceBlockingGate] Starting periodic health polling', {
      pollMs: SERVICE_GATE_POLL_MS,
    });
    const interval = window.setInterval(() => {
      void refreshStatus();
    }, SERVICE_GATE_POLL_MS);

    const onVisible = () => {
      if (document.visibilityState === 'visible') {
        console.info('[ServiceBlockingGate] App visible; forcing immediate status refresh');
        void refreshStatus();
      }
    };
    document.addEventListener('visibilitychange', onVisible);

    return () => {
      console.info('[ServiceBlockingGate] Stopping periodic health polling');
      window.clearInterval(interval);
      document.removeEventListener('visibilitychange', onVisible);
    };
  }, [refreshStatus]);

  const installed = useMemo(
    () => serviceStateText !== 'NotInstalled' && !serviceStateText.startsWith('Unknown'),
    [serviceStateText]
  );
  const serviceRunning = useMemo(() => serviceStateText === 'Running', [serviceStateText]);

  const runOperation = useCallback(
    async (label: string, op: () => Promise<unknown>) => {
      console.info('[ServiceBlockingGate] Running operation', { operation: label });
      setIsOperating(true);
      setOperatingLabel(label);
      setError(null);
      try {
        await op();
        console.info('[ServiceBlockingGate] Operation completed', { operation: label });
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        setError(message);
        console.error('[ServiceBlockingGate] Operation failed', {
          operation: label,
          error: message,
        });
      } finally {
        setIsOperating(false);
        setOperatingLabel(null);
        await refreshStatus();
      }
    },
    [refreshStatus]
  );

  const restartService = useCallback(async () => {
    console.info('[ServiceBlockingGate] Restart requested: stop -> start');
    await openhumanServiceStop();
    await openhumanServiceStart();
  }, []);

  if (gateStatus === 'ready') {
    return <>{children}</>;
  }

  // Stop/Restart/Uninstall require the service to be installed.
  // Install and Start are always available as recovery actions (never disabled).
  const canStop = !isOperating && installed && serviceRunning;
  const canRestart = !isOperating && installed;
  const canUninstall = !isOperating && installed;

  return (
    <div className="h-screen w-screen flex items-center justify-center bg-[#0a0d12] text-white px-6">
      <div className="w-full max-w-xl rounded-2xl border border-white/15 bg-black/30 p-6 space-y-4">
        <h1 className="text-xl font-semibold">OpenHuman Service Required</h1>
        <p className="text-sm text-white/70">
          The desktop service must be installed and running before the app can continue. Use the
          buttons below to set up or restart the service.
        </p>

        <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 text-sm">
          <div className="rounded-lg border border-white/10 bg-white/5 p-3">
            <div className="text-white/60">Service</div>
            <div className="font-medium">{serviceStateText}</div>
          </div>
          <div className="rounded-lg border border-white/10 bg-white/5 p-3">
            <div className="text-white/60">Agent Server</div>
            <div className="font-medium">{agentRunning ? 'Running' : 'Not Running'}</div>
          </div>
        </div>

        {isOperating ? (
          <div className="rounded-lg border border-blue-500/40 bg-blue-900/20 p-3 text-sm text-blue-300 flex items-center gap-2">
            <svg className="animate-spin h-4 w-4" viewBox="0 0 24 24" fill="none">
              <circle
                className="opacity-25"
                cx="12"
                cy="12"
                r="10"
                stroke="currentColor"
                strokeWidth="4"
              />
              <path
                className="opacity-75"
                fill="currentColor"
                d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
              />
            </svg>
            {operatingLabel ? `${operatingLabel}...` : 'Working...'}
          </div>
        ) : null}

        {error && !isOperating ? (
          <div className="rounded-lg border border-red-500/40 bg-red-900/20 p-3 text-sm text-red-300">
            {error}
          </div>
        ) : null}

        <div className="flex flex-wrap gap-2">
          <button
            onClick={() => {
              console.warn('[ServiceGate] INSTALL clicked', { isOperating, serviceStateText });
              if (isOperating) return;
              void runOperation('Installing service', () => openhumanServiceInstall());
            }}
            className="px-3 py-2 rounded-lg text-sm font-medium transition-colors bg-blue-600 hover:bg-blue-500 text-white cursor-pointer">
            {isOperating && operatingLabel === 'Installing service'
              ? 'Installing...'
              : 'Install Service'}
          </button>

          <button
            onClick={() => {
              console.warn('[ServiceGate] START clicked', { isOperating, serviceStateText });
              if (isOperating) return;
              void runOperation('Starting service', () => openhumanServiceStart());
            }}
            className="px-3 py-2 rounded-lg text-sm font-medium transition-colors bg-green-600 hover:bg-green-500 text-white cursor-pointer">
            {isOperating && operatingLabel === 'Starting service' ? 'Starting...' : 'Start Service'}
          </button>

          <button
            disabled={!canStop}
            onClick={() => {
              console.warn('[ServiceGate] STOP clicked', { isOperating, canStop });
              void runOperation('Stopping service', () => openhumanServiceStop());
            }}
            className={`px-3 py-2 rounded-lg text-sm font-medium transition-colors ${
              canStop
                ? 'bg-red-600 hover:bg-red-500 text-white cursor-pointer'
                : 'bg-white/5 text-white/30 cursor-not-allowed'
            }`}>
            Stop Service
          </button>

          <button
            disabled={!canRestart}
            onClick={() => {
              console.warn('[ServiceGate] RESTART clicked', { isOperating, canRestart });
              void runOperation('Restarting service', restartService);
            }}
            className={`px-3 py-2 rounded-lg text-sm font-medium transition-colors ${
              canRestart
                ? 'bg-cyan-700 hover:bg-cyan-600 text-white cursor-pointer'
                : 'bg-white/5 text-white/30 cursor-not-allowed'
            }`}>
            Restart Service
          </button>

          <button
            disabled={!canUninstall}
            onClick={() => {
              console.warn('[ServiceGate] UNINSTALL clicked', { isOperating, canUninstall });
              void runOperation('Uninstalling service', () => openhumanServiceUninstall());
            }}
            className={`px-3 py-2 rounded-lg text-sm font-medium transition-colors ${
              canUninstall
                ? 'bg-amber-700 hover:bg-amber-600 text-white cursor-pointer'
                : 'bg-white/5 text-white/30 cursor-not-allowed'
            }`}>
            Uninstall Service
          </button>

          <button
            onClick={() => {
              console.warn('[ServiceGate] REFRESH clicked');
              void refreshStatus({ showChecking: true, clearError: true });
            }}
            className="px-3 py-2 rounded-lg text-sm font-medium transition-colors bg-white/10 hover:bg-white/20 text-white cursor-pointer">
            Refresh
          </button>
        </div>
      </div>
    </div>
  );
};

export default ServiceBlockingGate;
