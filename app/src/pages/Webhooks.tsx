import ComposeioTriggerHistory from '../components/webhooks/ComposeioTriggerHistory';
import { useComposeioTriggerHistory } from '../hooks/useComposeioTriggerHistory';

export default function Webhooks() {
  const { archiveDir, currentDayFile, entries, loading, error, coreConnected, refresh } =
    useComposeioTriggerHistory(100);

  if (loading && entries.length === 0) {
    return (
      <div className="h-full flex items-center justify-center p-4 pt-6">
        <div className="flex flex-col items-center gap-3">
          <div className="h-8 w-8 animate-spin rounded-full border-2 border-stone-300 border-t-primary-500" />
          <span className="text-sm text-stone-500">Loading ComposeIO trigger history…</span>
        </div>
      </div>
    );
  }

  return (
    <div className="h-full overflow-y-auto p-4 pt-6">
      <div className="max-w-2xl mx-auto space-y-4">
        {/* Connection status */}
        <div className="flex flex-wrap items-center gap-3">
          <h2 className="text-xl font-semibold text-stone-900">ComposeIO Triggers</h2>
          <span
            className={`inline-flex items-center gap-1.5 px-2.5 py-1 text-xs font-medium rounded-full ${
              coreConnected ? 'bg-sage-100 text-sage-700' : 'bg-stone-100 text-stone-500'
            }`}>
            <span
              className={`w-1.5 h-1.5 rounded-full ${
                coreConnected ? 'bg-sage-500' : 'bg-stone-400'
              }`}
            />
            {coreConnected ? 'Connected' : 'Disconnected'}
          </span>
          <button
            type="button"
            onClick={() => void refresh()}
            className="rounded-full border border-stone-200 bg-white px-3 py-1.5 text-xs font-medium text-stone-700 transition hover:border-stone-300 hover:bg-stone-50">
            Refresh
          </button>
        </div>

        {error && <div className="p-3 rounded-lg bg-coral-50 text-coral-700 text-sm">{error}</div>}

        <div className="bg-white rounded-2xl shadow-soft border border-stone-200 p-6">
          <div className="space-y-3">
            <h3 className="text-lg font-semibold text-stone-900">Archive</h3>
            <p className="text-sm text-stone-600">
              Every ComposeIO trigger is appended to a daily JSONL file. Files are labeled by UTC
              day.
            </p>
            <div className="space-y-2 rounded-2xl border border-stone-200 bg-stone-50 p-4">
              <div>
                <div className="text-xs uppercase tracking-wide text-stone-400">
                  Archive Directory
                </div>
                <div className="font-mono text-xs break-all text-stone-700">
                  {archiveDir ?? 'Not available yet'}
                </div>
              </div>
              <div>
                <div className="text-xs uppercase tracking-wide text-stone-400">
                  Today&apos;s File
                </div>
                <div className="font-mono text-xs break-all text-stone-700">
                  {currentDayFile ?? 'Not available yet'}
                </div>
              </div>
            </div>
          </div>
        </div>

        <div className="bg-white rounded-2xl shadow-soft border border-stone-200 p-6">
          <ComposeioTriggerHistory entries={entries} />
        </div>
      </div>
    </div>
  );
}
