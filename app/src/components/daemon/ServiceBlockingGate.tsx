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
type RefreshOptions = {
  showChecking?: boolean;
  clearError?: boolean;
};

const normalizeServiceState = (state: ServiceState | undefined): string => {
  if (!state) return 'Unknown';
  if (typeof state === 'string') return state;
  if ('Unknown' in state) return `Unknown(${state.Unknown})`;
  return 'Unknown';
};

const ServiceBlockingGate = ({ children }: ServiceBlockingGateProps) => {
  const [gateStatus, setGateStatus] = useState<GateStatus>('checking');
  const [serviceStateText, setServiceStateText] = useState('Unknown');
  const [agentRunning, setAgentRunning] = useState(false);
  const [isOperating, setIsOperating] = useState(false);
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
      const [service, agent] = await Promise.all([
        openhumanServiceStatus(),
        openhumanAgentServerStatus(),
      ]);
      const serviceState = service?.result?.state;
      const normalized = normalizeServiceState(serviceState);
      const serviceRunning = normalized === 'Running';
      const agentIsRunning = !!agent?.result?.running;

      setServiceStateText(prev => (prev === normalized ? prev : normalized));
      setAgentRunning(prev => (prev === agentIsRunning ? prev : agentIsRunning));
      setGateStatus(prev => {
        const next = serviceRunning && agentIsRunning ? 'ready' : 'blocked';
        return prev === next ? prev : next;
      });
      setError(prev => (prev ? null : prev));
      console.info('[ServiceBlockingGate] Status refreshed', {
        serviceState: normalized,
        agentRunning: agentIsRunning,
        nextGateStatus: serviceRunning && agentIsRunning ? 'ready' : 'blocked',
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

  const installed = useMemo(() => serviceStateText !== 'NotInstalled', [serviceStateText]);
  const serviceRunning = useMemo(() => serviceStateText === 'Running', [serviceStateText]);

  const runOperation = useCallback(
    async (op: () => Promise<unknown>) => {
      const opName = op.name || 'anonymous-operation';
      console.info('[ServiceBlockingGate] Running operation', { operation: opName });
      setIsOperating(true);
      setError(null);
      try {
        await op();
        console.info('[ServiceBlockingGate] Operation completed', { operation: opName });
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        setError(message);
        console.error('[ServiceBlockingGate] Operation failed', {
          operation: opName,
          error: message,
        });
      } finally {
        setIsOperating(false);
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

  return (
    <div className="h-screen w-screen flex items-center justify-center bg-[#0a0d12] text-white px-6">
      <div className="w-full max-w-xl rounded-2xl border border-white/15 bg-black/30 p-6 space-y-4">
        <h1 className="text-xl font-semibold">OpenHuman Service Required</h1>
        <p className="text-sm text-white/70">
          The desktop service must be installed and running before the app can continue.
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

        {error ? (
          <div className="rounded-lg border border-red-500/40 bg-red-900/20 p-3 text-sm text-red-300">
            {error}
          </div>
        ) : null}

        <div className="flex flex-wrap gap-2">
          <button
            disabled={isOperating || installed}
            onClick={() => void runOperation(() => openhumanServiceInstall())}
            className="px-3 py-2 rounded-lg text-sm bg-blue-600 hover:bg-blue-700 disabled:bg-gray-700 disabled:text-gray-400">
            Install Service
          </button>

          <button
            disabled={isOperating || !installed || (serviceRunning && agentRunning)}
            onClick={() => void runOperation(() => openhumanServiceStart())}
            className="px-3 py-2 rounded-lg text-sm bg-green-600 hover:bg-green-700 disabled:bg-gray-700 disabled:text-gray-400">
            Start Service
          </button>

          <button
            disabled={isOperating || !installed || !serviceRunning}
            onClick={() => void runOperation(() => openhumanServiceStop())}
            className="px-3 py-2 rounded-lg text-sm bg-red-600 hover:bg-red-700 disabled:bg-gray-700 disabled:text-gray-400">
            Stop Service
          </button>

          <button
            disabled={isOperating || !installed}
            onClick={() => void runOperation(restartService)}
            className="px-3 py-2 rounded-lg text-sm bg-cyan-700 hover:bg-cyan-800 disabled:bg-gray-700 disabled:text-gray-400">
            Restart Service
          </button>

          <button
            disabled={isOperating || !installed}
            onClick={() => void runOperation(() => openhumanServiceUninstall())}
            className="px-3 py-2 rounded-lg text-sm bg-amber-700 hover:bg-amber-800 disabled:bg-gray-700 disabled:text-gray-400">
            Uninstall Service
          </button>

          <button
            disabled={isOperating}
            onClick={() => void refreshStatus({ showChecking: true, clearError: true })}
            className="px-3 py-2 rounded-lg text-sm bg-gray-700 hover:bg-gray-600 disabled:bg-gray-700/60 disabled:text-gray-400">
            Refresh
          </button>
        </div>
      </div>
    </div>
  );
};

export default ServiceBlockingGate;
