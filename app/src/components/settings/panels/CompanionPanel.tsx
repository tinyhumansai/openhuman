import { useCallback, useEffect, useState } from 'react';

import { callCoreRpc } from '../../../services/coreRpcClient';
import type {
  CompanionConfig,
  CompanionSessionStatus,
  StartCompanionSessionResult,
  StopCompanionSessionResult,
} from '../../../store/companionSlice';
import { useAppSelector } from '../../../store/hooks';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const CompanionPanel = () => {
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const companionState = useAppSelector(state => state.companion.state);

  const [status, setStatus] = useState<CompanionSessionStatus | null>(null);
  const [config, setConfig] = useState<CompanionConfig | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isStarting, setIsStarting] = useState(false);
  const [isStopping, setIsStopping] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const fetchStatus = useCallback(async () => {
    try {
      const result = await callCoreRpc<CompanionSessionStatus>({
        method: 'openhuman.companion_status',
      });
      setStatus(result);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  const fetchConfig = useCallback(async () => {
    try {
      const result = await callCoreRpc<CompanionConfig>({
        method: 'openhuman.companion_config_get',
      });
      setConfig(result);
    } catch {
      // Config fetch is best-effort — defaults shown if unavailable.
    }
  }, []);

  useEffect(() => {
    const load = async () => {
      setIsLoading(true);
      await Promise.all([fetchStatus(), fetchConfig()]);
      setIsLoading(false);
    };
    void load();
  }, [fetchStatus, fetchConfig]);

  // Poll status while panel is open.
  useEffect(() => {
    const id = window.setInterval(() => void fetchStatus(), 3000);
    return () => window.clearInterval(id);
  }, [fetchStatus]);

  const handleStart = async () => {
    setIsStarting(true);
    setError(null);
    try {
      await callCoreRpc<StartCompanionSessionResult>({
        method: 'openhuman.companion_start_session',
        params: { consent: true, ttl_secs: config?.ttl_secs ?? 3600 },
      });
      await fetchStatus();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsStarting(false);
    }
  };

  const handleStop = async () => {
    setIsStopping(true);
    setError(null);
    try {
      await callCoreRpc<StopCompanionSessionResult>({
        method: 'openhuman.companion_stop_session',
        params: { reason: 'user_requested' },
      });
      await fetchStatus();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsStopping(false);
    }
  };

  const sessionActive = status?.active ?? false;

  return (
    <div>
      <SettingsHeader
        title="Desktop Companion"
        showBackButton
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="space-y-4 p-4">
        {/* Status */}
        <div className="flex items-center justify-between">
          <div>
            <p className="text-sm font-medium text-stone-800">Session</p>
            <p className="text-xs text-stone-500">
              {isLoading ? 'Loading…' : sessionActive ? `Active — ${companionState}` : 'Inactive'}
            </p>
          </div>
          <div>
            {sessionActive ? (
              <button
                type="button"
                onClick={handleStop}
                disabled={isStopping}
                className="rounded-lg bg-red-50 px-3 py-1.5 text-sm font-medium text-red-700 hover:bg-red-100 disabled:opacity-50">
                {isStopping ? 'Stopping…' : 'Stop Session'}
              </button>
            ) : (
              <button
                type="button"
                onClick={handleStart}
                disabled={isStarting || isLoading}
                className="rounded-lg bg-blue-50 px-3 py-1.5 text-sm font-medium text-blue-700 hover:bg-blue-100 disabled:opacity-50">
                {isStarting ? 'Starting…' : 'Start Session'}
              </button>
            )}
          </div>
        </div>

        {/* Session details */}
        {sessionActive && status && (
          <div className="rounded-lg bg-stone-50 p-3 text-xs text-stone-600 space-y-1">
            <p>
              Session ID: <span className="font-mono">{status.session_id?.slice(0, 8)}…</span>
            </p>
            <p>Turns: {status.turn_count}</p>
            {status.remaining_ms != null && (
              <p>
                Remaining: {Math.floor(status.remaining_ms / 60000)}m{' '}
                {Math.floor((status.remaining_ms % 60000) / 1000)}s
              </p>
            )}
          </div>
        )}

        {/* Config */}
        {config && (
          <div className="space-y-3 border-t border-stone-100 pt-4">
            <p className="text-xs font-medium uppercase tracking-wide text-stone-400">
              Configuration
            </p>
            <div className="flex items-center justify-between">
              <span className="text-sm text-stone-700">Hotkey</span>
              <span className="rounded bg-stone-100 px-2 py-0.5 font-mono text-xs text-stone-600">
                {config.hotkey}
              </span>
            </div>
            <div className="flex items-center justify-between">
              <span className="text-sm text-stone-700">Activation Mode</span>
              <span className="text-xs text-stone-500">{config.activation_mode}</span>
            </div>
            <div className="flex items-center justify-between">
              <span className="text-sm text-stone-700">Session TTL</span>
              <span className="text-xs text-stone-500">{config.ttl_secs}s</span>
            </div>
            <div className="flex items-center justify-between">
              <span className="text-sm text-stone-700">Screen Capture</span>
              <span className="text-xs text-stone-500">
                {config.capture_screen ? 'Enabled' : 'Disabled'}
              </span>
            </div>
            <div className="flex items-center justify-between">
              <span className="text-sm text-stone-700">App Context</span>
              <span className="text-xs text-stone-500">
                {config.include_app_context ? 'Enabled' : 'Disabled'}
              </span>
            </div>
          </div>
        )}

        {/* Error */}
        {error && <div className="rounded-lg bg-red-50 p-3 text-xs text-red-700">{error}</div>}
      </div>
    </div>
  );
};

export default CompanionPanel;
