import { useT } from '../../lib/i18n/I18nContext';
import type { QueuedFollowup } from '../../store/chatRuntimeSlice';

export interface QueuedFollowupsProps {
  /** Follow-ups queued for the current thread while a turn is streaming. */
  items: QueuedFollowup[];
  /** Dismiss every queued follow-up (clears the backend run-queue too). */
  onClear: () => void;
}

/**
 * Compact strip rendered above the composer while one or more follow-up
 * messages are queued behind a streaming turn. Lets the user see what they
 * queued (so a typed follow-up is never silently lost) and clear the queue
 * before the backend dispatches them. Send/queueing happens in the composer;
 * this is a read-only surface plus a single clear action.
 */
export default function QueuedFollowups({ items, onClear }: QueuedFollowupsProps) {
  const { t } = useT();
  if (items.length === 0) return null;

  return (
    <div
      data-testid="queued-followups"
      className="mb-2 rounded-xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2">
      <div className="flex items-center justify-between gap-2 mb-1.5">
        <span className="text-xs font-medium text-stone-500 dark:text-neutral-400">
          {t('chat.queuedFollowups.label')} · {items.length}
        </span>
        <button
          type="button"
          data-analytics-id="chat-queued-followups-clear"
          onClick={onClear}
          className="text-xs font-medium text-stone-500 dark:text-neutral-400 hover:text-coral-500 dark:hover:text-coral-400 transition-colors">
          {t('chat.queuedFollowups.clear')}
        </button>
      </div>
      <ul className="flex flex-col gap-1">
        {items.map(item => (
          <li
            key={item.message.id}
            className="truncate text-sm text-stone-700 dark:text-neutral-200"
            title={item.label}>
            {item.label}
          </li>
        ))}
      </ul>
    </div>
  );
}
