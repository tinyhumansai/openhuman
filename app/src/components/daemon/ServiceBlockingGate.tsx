import { useCallback, useEffect, useMemo, useState } from 'react';

import {
  isTauri,
  openhumanAgentServerStatus,
  openhumanServiceInstall,
  openhumanServiceStart,
  openhumanServiceStatus,
  openhumanServiceStop,
  type ServiceState,
} from '../../utils/tauriCommands';

interface ServiceBlockingGateProps {
  children: React.ReactNode;
}

type GateStatus = 'checking' | 'ready' | 'blocked';

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

  const refreshStatus = useCallback(async () => {
    if (!isTauri()) {
      setGateStatus('ready');
      return;
    }

    setError(null);
    setGateStatus('checking');

    try {
      const [service, agent] = await Promise.all([openhumanServiceStatus(), openhumanAgentServerStatus()]);
      const serviceState = service?.result?.state;
      const normalized = normalizeServiceState(serviceState);
      const serviceRunning = normalized === 'Running';
      const agentIsRunning = !!agent?.result?.running;

      setServiceStateText(normalized);
      setAgentRunning(agentIsRunning);
      setGateStatus(serviceRunning && agentIsRunning ? 'ready' : 'blocked');
    } catch (err) {
      setServiceStateText('Unknown');
      setAgentRunning(false);
      setGateStatus('blocked');
      setError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  useEffect(() => {
    void refreshStatus();
  }, [refreshStatus]);

  const installed = useMemo(() => serviceStateText !== 'NotInstalled', [serviceStateText]);
  const serviceRunning = useMemo(() => serviceStateText === 'Running', [serviceStateText]);

  const runOperation = useCallback(
    async (op: () => Promise<unknown>) => {
      setIsOperating(true);
      setError(null);
      try {
        await op();
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        setIsOperating(false);
        await refreshStatus();
      }
    },
    [refreshStatus]
  );

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
            disabled={isOperating}
            onClick={() => void refreshStatus()}
            className="px-3 py-2 rounded-lg text-sm bg-gray-700 hover:bg-gray-600 disabled:bg-gray-700/60 disabled:text-gray-400">
            Refresh
          </button>
        </div>
      </div>
    </div>
  );
};

export default ServiceBlockingGate;
