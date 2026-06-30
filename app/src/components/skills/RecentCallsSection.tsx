import { useCallback, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import {
  getMeetCallDetail,
  type MeetCallDetail,
  type MeetCallRecord,
  type MeetCallSummary,
  type MeetCallTranscriptLine,
} from '../../services/meetCallService';

/**
 * Recent-calls history shown under the meeting-bot join form. Renders the
 * loading / empty / populated states and one row per completed call (meeting
 * code, relative time, turn count, duration, owner, and participants).
 *
 * Each row is expandable: on first expand it lazily fetches the call's
 * transcript + summary via `meet_agent_get_call_detail` so the list payload
 * stays lean. Older calls recorded before the feature have no detail and show
 * a "nothing captured" state.
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
      className="mt-4 border-t border-line pt-4">
      <div className="flex items-baseline justify-between">
        <h3 className="text-[11px] font-semibold uppercase tracking-wide text-content-muted">
          {t('skills.meetingBots.recentCallsHeading')}
          {rows && rows.length > 0 && (
            <span className="ml-1 text-content-faint normal-case font-normal">
              ({rows.length})
            </span>
          )}
        </h3>
      </div>

      {error && <p className="mt-2 text-[11px] text-coral-600 dark:text-coral-400">{error}</p>}

      {rows === null ? (
        <p className="mt-2 text-[11px] text-content-faint">
          {t('skills.meetingBots.recentCallsLoading')}
        </p>
      ) : rows.length === 0 ? (
        <p className="mt-2 text-[11px] text-content-faint">
          {t('skills.meetingBots.recentCallsEmpty')}
        </p>
      ) : (
        <ul className="mt-2 max-h-72 space-y-1 overflow-y-auto pr-1">
          {rows.map(call => (
            <RecentCallRow key={call.request_id} call={call} />
          ))}
        </ul>
      )}
    </section>
  );
}

type DetailStatus = 'idle' | 'loading' | 'loaded' | 'error';

/**
 * True when `detail` carries a non-empty generated summary. The summary lands
 * asynchronously after the transcript at call-end, so this gates whether a
 * re-expand should refetch (still pending) or reuse the cache (already present).
 */
function hasSummaryDetail(detail: MeetCallDetail | null): boolean {
  const summary = detail?.summary;
  return (
    !!summary &&
    (summary.headline.trim().length > 0 ||
      summary.key_points.length > 0 ||
      summary.action_items.length > 0)
  );
}

function RecentCallRow({ call }: { call: MeetCallRecord }) {
  const { t } = useT();
  const [expanded, setExpanded] = useState(false);
  const [status, setStatus] = useState<DetailStatus>('idle');
  const [detail, setDetail] = useState<MeetCallDetail | null>(null);

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

  const loadDetail = useCallback(async () => {
    setStatus('loading');
    try {
      const result = await getMeetCallDetail(call.request_id);
      setDetail(result);
      setStatus('loaded');
    } catch (err) {
      console.error('[recent-calls] failed to load call detail', call.request_id, err);
      setStatus('error');
    }
  }, [call.request_id]);

  const toggle = useCallback(() => {
    setExpanded(prev => {
      const next = !prev;
      // Lazy-load on first expand. The transcript is persisted at call-end but
      // the summary is generated asynchronously and patched in moments later, so
      // re-expanding a row whose cached detail still lacks a summary refetches to
      // pick it up. A complete (summary-present) detail is reused as-is.
      if (next && (status === 'idle' || (status === 'loaded' && !hasSummaryDetail(detail)))) {
        void loadDetail();
      }
      return next;
    });
  }, [status, detail, loadDetail]);

  return (
    <li className="rounded-lg text-[11px] text-content-secondary">
      <button
        type="button"
        onClick={toggle}
        aria-expanded={expanded}
        className="w-full rounded-lg px-2 py-1.5 text-left hover:bg-surface-muted dark:hover:bg-surface-muted/40">
        <div className="flex items-center justify-between gap-2">
          <span className="flex min-w-0 items-center gap-1">
            <Chevron expanded={expanded} />
            <span className="truncate font-mono text-content">
              {meetingCode}
            </span>
          </span>
          <span className="shrink-0 text-content-faint">
            {formatRelativeTime(call.started_at_ms)}
          </span>
        </div>
        <div className="mt-0.5 flex items-center gap-3 pl-4 text-[10px] text-content-muted">
          <span>
            {t(
              call.turn_count === 1
                ? 'skills.meetingBots.recentCallTurnSingular'
                : 'skills.meetingBots.recentCallTurnPlural'
            ).replace('{count}', String(call.turn_count))}
          </span>
          <span>
            {t('skills.meetingBots.recentCallDuration').replace('{seconds}', String(duration))}
          </span>
          {owner && (
            <span className="truncate">
              {t('skills.meetingBots.recentCallAddedBy').replace('{name}', owner)}
            </span>
          )}
        </div>
        {participants.length > 0 && (
          <div className="mt-0.5 truncate pl-4 text-[10px] text-content-muted">
            {t('skills.meetingBots.recentCallParticipants').replace(
              '{names}',
              participants.join(', ')
            )}
          </div>
        )}
      </button>

      {expanded && (
        <div className="px-2 pb-2 pl-6">
          <RecentCallDetailBody status={status} detail={detail} onRetry={loadDetail} />
        </div>
      )}
    </li>
  );
}

function RecentCallDetailBody({
  status,
  detail,
  onRetry,
}: {
  status: DetailStatus;
  detail: MeetCallDetail | null;
  onRetry: () => void;
}) {
  const { t } = useT();

  if (status === 'idle' || status === 'loading') {
    return (
      <p className="text-[10px] text-content-faint">
        {t('skills.meetingBots.callDetailLoading')}
      </p>
    );
  }

  if (status === 'error') {
    return (
      <p className="text-[10px] text-coral-600 dark:text-coral-400">
        {t('skills.meetingBots.callDetailError')}{' '}
        <button
          type="button"
          onClick={onRetry}
          className="underline underline-offset-2 hover:text-coral-700 dark:hover:text-coral-300">
          {t('skills.meetingBots.callDetailRetry')}
        </button>
      </p>
    );
  }

  const summary = detail?.summary ?? null;
  const transcript = detail?.transcript ?? [];
  const hasSummary = hasSummaryDetail(detail);

  if (!hasSummary && transcript.length === 0) {
    return (
      <p className="text-[10px] text-content-faint">
        {t('skills.meetingBots.callDetailEmpty')}
      </p>
    );
  }

  return (
    <div className="space-y-3">
      {hasSummary && summary && <CallSummary summary={summary} />}
      {transcript.length > 0 && <CallTranscript lines={transcript} />}
    </div>
  );
}

function CallSummary({ summary }: { summary: MeetCallSummary }) {
  const { t } = useT();
  return (
    <div className="space-y-1.5">
      <SectionLabel>{t('skills.meetingBots.callSummaryHeading')}</SectionLabel>
      {summary.headline.trim() && (
        <p className="text-[11px] text-content-secondary">{summary.headline}</p>
      )}
      {summary.key_points.length > 0 && (
        <div>
          <p className="text-[10px] font-medium text-content-muted">
            {t('skills.meetingBots.callKeyPointsHeading')}
          </p>
          <ul className="mt-0.5 list-disc space-y-0.5 pl-4 text-[10px] text-content-secondary">
            {summary.key_points.map((point, i) => (
              <li key={i}>{point}</li>
            ))}
          </ul>
        </div>
      )}
      {summary.action_items.length > 0 && (
        <div>
          <p className="text-[10px] font-medium text-content-muted">
            {t('skills.meetingBots.callActionItemsHeading')}
          </p>
          <ul className="mt-0.5 space-y-0.5 text-[10px] text-content-secondary">
            {summary.action_items.map((item, i) => {
              const meta = [
                item.assignee?.trim() || undefined,
                item.kind === 'executable' ? item.tool_name?.trim() || undefined : undefined,
              ].filter(Boolean);
              return (
                <li key={i} className="flex gap-1">
                  <span aria-hidden="true">•</span>
                  <span>
                    {item.description}
                    {meta.length > 0 && (
                      <span className="text-content-faint">
                        {' '}
                        ({meta.join(' · ')})
                      </span>
                    )}
                  </span>
                </li>
              );
            })}
          </ul>
        </div>
      )}
    </div>
  );
}

function CallTranscript({ lines }: { lines: MeetCallTranscriptLine[] }) {
  const { t } = useT();
  return (
    <div className="space-y-1">
      <SectionLabel>{t('skills.meetingBots.callTranscriptHeading')}</SectionLabel>
      <div className="max-h-48 space-y-0.5 overflow-y-auto rounded-md bg-surface-muted p-2">
        {lines.map((line, i) => (
          <p
            key={i}
            className={
              line.role === 'assistant'
                ? 'text-[10px] text-ocean-700 dark:text-ocean-300'
                : 'text-[10px] text-content-secondary'
            }>
            {line.content}
          </p>
        ))}
      </div>
    </div>
  );
}

function SectionLabel({ children }: { children: React.ReactNode }) {
  return (
    <p className="text-[10px] font-semibold uppercase tracking-wide text-content-muted">
      {children}
    </p>
  );
}

function Chevron({ expanded }: { expanded: boolean }) {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 16 16"
      className={`h-3 w-3 shrink-0 text-content-faint transition-transform ${
        expanded ? 'rotate-90' : ''
      }`}
      fill="none"
      stroke="currentColor"
      strokeWidth="2">
      <path d="M6 4l4 4-4 4" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
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
