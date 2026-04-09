import type { AutocompleteStatus } from '../../../../utils/tauriCommands';

interface AppFilterSectionProps {
  status: AutocompleteStatus | null;
  isLoading: boolean;
  contextOverride: string;
  focusDebug: string;
  logs: string[];
  message: string | null;
  error: string | null;
  onSetContextOverride: (value: string) => void;
  onRefreshStatus: () => void;
  onStart: () => void;
  onStop: () => void;
  onTestCurrent: () => void;
  onAcceptSuggestion: () => void;
  onDebugFocus: () => void;
  onClearLogs: () => void;
}

const AppFilterSection = ({
  status,
  isLoading,
  contextOverride,
  focusDebug,
  logs,
  message,
  error,
  onSetContextOverride,
  onRefreshStatus,
  onStart,
  onStop,
  onTestCurrent,
  onAcceptSuggestion,
  onDebugFocus,
  onClearLogs,
}: AppFilterSectionProps) => {
  return (
    <>
      <section className="rounded-2xl border border-stone-200 bg-white p-4 space-y-3">
        <h3 className="text-sm font-semibold text-stone-900">Runtime</h3>
        <div className="text-sm text-stone-700 space-y-1">
          <div>Platform supported: {status?.platform_supported ? 'yes' : 'no'}</div>
          <div>Enabled: {status?.enabled ? 'yes' : 'no'}</div>
          <div>Running: {status?.running ? 'yes' : 'no'}</div>
          <div>Phase: {status?.phase ?? 'unknown'}</div>
          <div>Debounce: {status?.debounce_ms ?? 0}ms</div>
          <div>Model: {status?.model_id ?? 'n/a'}</div>
          <div>App: {status?.app_name ?? 'n/a'}</div>
          <div>Last error: {status?.last_error ?? 'none'}</div>
          <div>Current suggestion: {status?.suggestion?.value ?? 'none'}</div>
        </div>
        <div className="flex gap-2">
          <button
            type="button"
            onClick={onRefreshStatus}
            disabled={isLoading}
            className="rounded-lg border border-stone-300 bg-stone-100 px-3 py-2 text-sm text-stone-700 disabled:opacity-50">
            {isLoading ? 'Refreshing…' : 'Refresh Status'}
          </button>
          <button
            type="button"
            onClick={onStart}
            disabled={!status?.platform_supported || Boolean(status?.running)}
            className="rounded-lg border border-green-500/60 bg-green-50 px-3 py-2 text-sm text-green-700 disabled:opacity-50">
            Start
          </button>
          <button
            type="button"
            onClick={onStop}
            disabled={!status?.running}
            className="rounded-lg border border-red-500/60 bg-red-50 px-3 py-2 text-sm text-red-600 disabled:opacity-50">
            Stop
          </button>
        </div>
      </section>

      <section className="rounded-2xl border border-stone-200 bg-white p-4 space-y-3">
        <h3 className="text-sm font-semibold text-stone-900">Test</h3>
        <div className="space-y-1">
          <div className="text-xs text-stone-600">Context Override (optional)</div>
          <textarea
            value={contextOverride}
            onChange={event => onSetContextOverride(event.target.value)}
            rows={3}
            className="w-full rounded border border-stone-200 bg-stone-50 p-2 text-xs text-stone-700"
          />
        </div>
        <div className="flex gap-2">
          <button
            type="button"
            onClick={onTestCurrent}
            className="rounded-lg border border-primary-500/60 bg-primary-50 px-3 py-2 text-sm text-primary-600">
            Get Suggestion
          </button>
          <button
            type="button"
            onClick={onAcceptSuggestion}
            className="rounded-lg border border-emerald-500/60 bg-emerald-50 px-3 py-2 text-sm text-emerald-700">
            Accept Suggestion
          </button>
          <button
            type="button"
            onClick={onDebugFocus}
            className="rounded-lg border border-amber-500/60 bg-amber-50 px-3 py-2 text-sm text-amber-700">
            Debug Focus
          </button>
        </div>
        {focusDebug && (
          <pre className="max-h-48 overflow-auto rounded-xl border border-stone-200 bg-stone-50 p-2 text-xs text-stone-700">
            {focusDebug}
          </pre>
        )}
      </section>

      <section className="rounded-2xl border border-stone-200 bg-white p-4 space-y-3">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-semibold text-stone-900">Live Logs</h3>
          <button
            type="button"
            onClick={onClearLogs}
            className="rounded-lg border border-stone-300 bg-stone-100 px-3 py-1.5 text-xs text-stone-700">
            Clear
          </button>
        </div>
        <pre className="max-h-56 overflow-auto rounded-xl border border-stone-200 bg-stone-50 p-2 text-xs text-stone-700">
          {logs.length > 0 ? logs.join('\n') : 'No logs yet.'}
        </pre>
      </section>

      {message && <div className="text-xs text-green-700">{message}</div>}
      {error && <div className="text-xs text-red-600">{error}</div>}
    </>
  );
};

export default AppFilterSection;
