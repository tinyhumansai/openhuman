import { formatTriggerLabel } from '../../lib/composio/formatters';
import { useT } from '../../lib/i18n/I18nContext';
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
    const formatted = JSON.stringify(payload, null, 2);
    return formatted ?? String(payload);
  } catch {
    return String(payload);
  }
}

export default function ComposeioTriggerHistory({ entries }: ComposeioTriggerHistoryProps) {
  const { t } = useT();
  if (entries.length === 0) {
    return (
      <div className="space-y-3">
        <h3 className="text-lg font-semibold text-stone-900 dark:text-neutral-100">
          {t('webhooks.composioHistory.title')}
        </h3>
        <p className="rounded-xl border border-dashed border-stone-200 dark:border-neutral-700 bg-stone-50 dark:bg-neutral-800/60 px-4 py-6 text-center text-sm text-stone-500 dark:text-neutral-400">
          {t('webhooks.composioHistory.empty')}
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <h3 className="text-lg font-semibold text-stone-900">
        {t('webhooks.composioHistory.title')}{' '}
        <span className="text-sm font-normal text-stone-400 dark:text-neutral-500">
          ({entries.length})
        </span>
      </h3>
      <div className="space-y-3">
        {entries.map(entry => (
          <article
            key={`${entry.metadata_uuid}-${entry.received_at_ms}`}
            className="rounded-2xl border border-stone-200 dark:border-neutral-800 bg-stone-50/60 dark:bg-neutral-800/60 p-4">
            <div className="flex flex-wrap items-center gap-2">
              <span className="rounded-full bg-primary-50 px-2.5 py-1 text-xs font-medium text-primary-700">
                {entry.toolkit}
              </span>
              <span className="rounded-full bg-sage-50 px-2.5 py-1 text-xs font-medium text-sage-700">
                {formatTriggerLabel(entry.trigger)}
              </span>
              <span className="text-xs text-stone-500 dark:text-neutral-400">
                {formatTimestamp(entry.received_at_ms)}
              </span>
            </div>

            <dl className="mt-3 grid gap-2 text-sm text-stone-700 dark:text-neutral-200 md:grid-cols-2">
              <div>
                <dt className="text-xs uppercase tracking-wide text-stone-400 dark:text-neutral-500">
                  {t('webhooks.composioHistory.metadataId')}
                </dt>
                <dd className="font-mono text-xs break-all">{entry.metadata_id}</dd>
              </div>
              <div>
                <dt className="text-xs uppercase tracking-wide text-stone-400 dark:text-neutral-500">
                  {t('webhooks.composioHistory.metadataUuid')}
                </dt>
                <dd className="font-mono text-xs break-all">{entry.metadata_uuid}</dd>
              </div>
            </dl>

            <div className="mt-3">
              <div className="mb-2 text-xs uppercase tracking-wide text-stone-400 dark:text-neutral-500">
                {t('webhooks.composioHistory.payload')}
              </div>
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
