import { useT } from '../../lib/i18n/I18nContext';
import { type MeetCallRecord } from '../../services/meetCallService';

/**
 * Recent-calls history shown under the meeting-bot join form. Renders the
 * loading / empty / populated states and one row per completed call (meeting
 * code, relative time, turn count, duration, owner, and participants).
 *
 * Extracted from `MeetingBotsCard` to keep that component within the repo's
 * ~500-line file-size guideline.
 */
export function RecentCallsSection({
  rows,
  error,
}: {
  rows: MeetCallRecord[] | null;
  error: string | null;
}) {
  const { t } = useT();
  return (
    <section
      aria-label={t('skills.meetingBots.recentCallsAriaLabel')}
      className="mt-4 border-t border-stone-200 dark:border-neutral-800 pt-4">
      <div className="flex items-baseline justify-between">
        <h3 className="text-[11px] font-semibold uppercase tracking-wide text-stone-500 dark:text-neutral-400">
          {t('skills.meetingBots.recentCallsHeading')}
          {rows && rows.length > 0 && (
            <span className="ml-1 text-stone-400 dark:text-neutral-500 normal-case font-normal">
              ({rows.length})
            </span>
          )}
        </h3>
      </div>

      {error && <p className="mt-2 text-[11px] text-coral-600 dark:text-coral-400">{error}</p>}

      {rows === null ? (
        <p className="mt-2 text-[11px] text-stone-400 dark:text-neutral-500">
          {t('skills.meetingBots.recentCallsLoading')}
        </p>
      ) : rows.length === 0 ? (
        <p className="mt-2 text-[11px] text-stone-400 dark:text-neutral-500">
          {t('skills.meetingBots.recentCallsEmpty')}
        </p>
      ) : (
        <ul className="mt-2 max-h-48 space-y-1 overflow-y-auto pr-1">
          {rows.map(call => (
            <RecentCallRow key={call.request_id} call={call} />
          ))}
        </ul>
      )}
    </section>
  );
}

function RecentCallRow({ call }: { call: MeetCallRecord }) {
  const { t } = useT();
  const meetingCode = (() => {
    try {
      const parsed = new URL(call.meet_url);
      const tail = parsed.pathname.replace(/^\/+/, '');
      return tail || call.meet_url;
    } catch {
      return call.meet_url || '(unknown URL)';
    }
  })();
  const duration = Math.max(0, Math.round(call.spoken_seconds + call.listened_seconds));
  const owner = call.owner_display_name?.trim();
  const participants = (call.participants ?? []).map(p => p.trim()).filter(Boolean);
  return (
    <li className="rounded-lg px-2 py-1.5 text-[11px] text-stone-700 dark:text-neutral-300 hover:bg-stone-50 dark:hover:bg-neutral-800/40">
      <div className="flex items-center justify-between gap-2">
        <span className="truncate font-mono text-stone-800 dark:text-neutral-200">
          {meetingCode}
        </span>
        <span className="shrink-0 text-stone-400 dark:text-neutral-500">
          {formatRelativeTime(call.started_at_ms)}
        </span>
      </div>
      <div className="mt-0.5 flex items-center gap-3 text-[10px] text-stone-500 dark:text-neutral-400">
        <span>
          {call.turn_count} turn{call.turn_count === 1 ? '' : 's'}
        </span>
        <span>{duration}s on call</span>
        {owner && (
          <span className="truncate">
            {t('skills.meetingBots.recentCallAddedBy').replace('{name}', owner)}
          </span>
        )}
      </div>
      {participants.length > 0 && (
        <div className="mt-0.5 truncate text-[10px] text-stone-500 dark:text-neutral-400">
          {t('skills.meetingBots.recentCallParticipants').replace('{names}', participants.join(', '))}
        </div>
      )}
    </li>
  );
}

function formatRelativeTime(ms: number): string {
  if (!ms) return '—';
  const diff = Date.now() - ms;
  if (diff < 0) return 'just now';
  const seconds = Math.floor(diff / 1000);
  if (seconds < 60) return 'just now';
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days === 1) return 'yesterday';
  if (days < 7) return `${days}d ago`;
  try {
    return new Date(ms).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
  } catch {
    return '—';
  }
}
