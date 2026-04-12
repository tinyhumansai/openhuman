import type { ComposioTriggerHistoryEntry } from '../../utils/tauriCommands';

interface ComposeioTriggerHistoryProps {
  entries: ComposioTriggerHistoryEntry[];
}

function formatTimestamp(ts: number): string {
  return new Date(ts).toLocaleString(undefined, {
    year: 'numeric',
    month: 'short',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  });
}

function formatPayload(payload: unknown): string {
  try {
    return JSON.stringify(payload, null, 2);
  } catch {
    return String(payload);
  }
}

export default function ComposeioTriggerHistory({ entries }: ComposeioTriggerHistoryProps) {
  if (entries.length === 0) {
    return (
      <div className="space-y-3">
        <h3 className="text-lg font-semibold text-stone-900">ComposeIO Trigger History</h3>
        <p className="rounded-xl border border-dashed border-stone-200 bg-stone-50 px-4 py-6 text-center text-sm text-stone-500">
          No ComposeIO triggers have been captured yet.
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <h3 className="text-lg font-semibold text-stone-900">
        ComposeIO Trigger History{' '}
        <span className="text-sm font-normal text-stone-400">({entries.length})</span>
      </h3>
      <div className="space-y-3">
        {entries.map(entry => (
          <article
            key={`${entry.metadata_uuid}-${entry.received_at_ms}`}
            className="rounded-2xl border border-stone-200 bg-stone-50/60 p-4">
            <div className="flex flex-wrap items-center gap-2">
              <span className="rounded-full bg-primary-50 px-2.5 py-1 text-xs font-medium text-primary-700">
                {entry.toolkit}
              </span>
              <span className="rounded-full bg-sage-50 px-2.5 py-1 text-xs font-medium text-sage-700">
                {entry.trigger}
              </span>
              <span className="text-xs text-stone-500">
                {formatTimestamp(entry.received_at_ms)}
              </span>
            </div>

            <dl className="mt-3 grid gap-2 text-sm text-stone-700 md:grid-cols-2">
              <div>
                <dt className="text-xs uppercase tracking-wide text-stone-400">Metadata ID</dt>
                <dd className="font-mono text-xs break-all">{entry.metadata_id}</dd>
              </div>
              <div>
                <dt className="text-xs uppercase tracking-wide text-stone-400">Metadata UUID</dt>
                <dd className="font-mono text-xs break-all">{entry.metadata_uuid}</dd>
              </div>
            </dl>

            <div className="mt-3">
              <div className="mb-2 text-xs uppercase tracking-wide text-stone-400">Payload</div>
              <pre className="max-h-64 overflow-auto rounded-xl bg-stone-900 px-3 py-3 text-xs text-stone-100">
                {formatPayload(entry.payload)}
              </pre>
            </div>
          </article>
        ))}
      </div>
    </div>
  );
}
