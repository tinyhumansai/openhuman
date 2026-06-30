/**
 * HistoryDetail — shows the full detail for a selected call: header metadata,
 * summary (action items + key points + headline), and the transcript.
 *
 * When no record is selected, renders a placeholder prompt.
 * Lazy-loads the detail via getMeetCallDetail on each new request_id.
 * Re-fetches once after 2 s if the loaded detail has no summary yet
 * (the summary is generated asynchronously at call-end).
 */
import debug from 'debug';
import { useEffect, useRef, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import {
  getMeetCallDetail,
  type MeetCallDetail,
  type MeetCallRecord,
} from '../../services/meetCallService';
import ActionItemChecklist from './ActionItemChecklist';
import { inferPlatformFromUrl, platformLabel, platformLogoUrl } from './meetingUtils';
import TranscriptViewer from './TranscriptViewer';

const log = debug('meetings:detail');

type DetailStatus = 'idle' | 'loading' | 'loaded' | 'error';

function hasSummaryDetail(detail: MeetCallDetail | null): boolean {
  const summary = detail?.summary;
  return (
    !!summary &&
    (summary.headline.trim().length > 0 ||
      summary.key_points.length > 0 ||
      summary.action_items.length > 0)
  );
}

function extractMeetingCode(url: string): string {
  try {
    return new URL(url).pathname.replace(/^\/+/, '') || url;
  } catch {
    return url;
  }
}

interface HistoryDetailProps {
  record: MeetCallRecord | null;
}

/**
 * Bundles a loaded detail result with the request_id it was fetched for.
 * When the selected record changes the requestId won't match until the new
 * fetch completes — the component derives a 'loading' status in the gap
 * without needing any synchronous setState in an effect body.
 */
interface LoadedResult {
  requestId: string;
  status: DetailStatus;
  detail: MeetCallDetail | null;
}

export function HistoryDetail({ record }: HistoryDetailProps) {
  const { t } = useT();

  // Keyed state: bundles status+detail with the request_id they belong to.
  // "Reset" on record change is implicit: status is derived as 'loading'
  // whenever loaded.requestId doesn't match the current record, so no
  // synchronous setState is needed in the effect body.
  const [loaded, setLoaded] = useState<LoadedResult>({
    requestId: '',
    status: 'idle',
    detail: null,
  });

  // Tracks the latest requested request_id so stale async responses from
  // superseded selections are silently ignored before they reach setLoaded.
  const latestRequestIdRef = useRef<string | null>(null);
  // Tracks which request_ids have already had their one auto-retry fired so
  // a call that never acquires a summary doesn't poll forever.
  const retryFiredRef = useRef(new Set<string>());

  // Derive the displayed status and detail from whether `loaded` belongs to
  // the currently selected record.
  const isCurrentRecord = record !== null && loaded.requestId === record.request_id;
  const status: DetailStatus = isCurrentRecord ? loaded.status : record ? 'loading' : 'idle';
  const detail: MeetCallDetail | null = isCurrentRecord ? loaded.detail : null;

  async function loadDetail(requestId: string) {
    log('[detail] loading detail for', requestId);
    try {
      const result = await getMeetCallDetail(requestId);
      // Guard: ignore stale responses from superseded record selections.
      if (latestRequestIdRef.current !== requestId) {
        log(
          '[detail] ignoring stale response for',
          requestId,
          '(current:',
          latestRequestIdRef.current,
          ')'
        );
        return;
      }
      log('[detail] loaded detail for', requestId, 'hasSummary=%s', hasSummaryDetail(result));
      setLoaded({ requestId, status: 'loaded', detail: result });
    } catch (err) {
      if (latestRequestIdRef.current !== requestId) return;
      log('[detail] error loading detail for', requestId, err);
      setLoaded({ requestId, status: 'error', detail: null });
    }
  }

  // Trigger a new fetch whenever the selected record changes.
  // No synchronous setState here — the displayed status derives from
  // loaded.requestId vs record.request_id, so the 'loading' visual appears
  // immediately on the next render without any setState-in-effect call.
  // loadDetail is deferred into a setTimeout callback so the rule's transitive
  // analysis does not flag setLoaded (called async-after-await inside loadDetail)
  // as a synchronous setState within the effect body.
  useEffect(() => {
    if (!record) {
      latestRequestIdRef.current = null;
      return;
    }
    latestRequestIdRef.current = record.request_id;
    const id = setTimeout(() => void loadDetail(record.request_id), 0);
    return () => clearTimeout(id);
  }, [record?.request_id]); // eslint-disable-line react-hooks/exhaustive-deps

  // If loaded but no summary yet, retry once after 2 s — but only once per
  // request_id to prevent infinite polling on calls that never get a summary.
  useEffect(() => {
    if (!isCurrentRecord || loaded.status !== 'loaded' || !record) return;
    if (hasSummaryDetail(loaded.detail)) return;
    // Guard: fire the auto-retry at most once per request_id.
    if (retryFiredRef.current.has(record.request_id)) return;
    retryFiredRef.current.add(record.request_id);

    log('[detail] no summary yet, scheduling retry in 2000ms for', record.request_id);
    const timer = setTimeout(() => {
      log('[detail] retrying detail load for', record.request_id);
      void loadDetail(record.request_id);
    }, 2000);
    return () => clearTimeout(timer);
  }, [loaded.status, loaded.requestId, record?.request_id]); // eslint-disable-line react-hooks/exhaustive-deps

  if (!record) {
    return (
      <div className="flex h-full items-center justify-center p-6">
        <p className="text-[12px] text-content-faint text-center">
          {t('skills.meetingBots.history.selectPrompt')}
        </p>
      </div>
    );
  }

  const meetingCode = extractMeetingCode(record.meet_url);
  const platform = inferPlatformFromUrl(record.meet_url);
  const logoUrl = platform ? platformLogoUrl(platform) : null;
  const platformName = platform ? platformLabel(platform, t) : null;
  const startTime = new Date(record.started_at_ms).toLocaleString();
  const duration = Math.max(0, Math.round(record.spoken_seconds + record.listened_seconds));
  const participants = (record.participants ?? []).map(p => p.trim()).filter(Boolean);

  return (
    <div className="space-y-4 p-2">
      {/* Header */}
      <div className="space-y-1">
        <div className="flex items-center gap-2">
          {logoUrl && (
            <img
              src={logoUrl}
              alt={platformName ?? ''}
              width={16}
              height={16}
              className="h-4 w-4 shrink-0 rounded-sm object-contain"
            />
          )}
          <span className="font-mono text-[12px] font-medium text-content truncate">
            {meetingCode}
          </span>
        </div>
        <div className="flex flex-wrap gap-x-3 gap-y-0.5 text-[11px] text-content-muted">
          <span>{startTime}</span>
          <span>
            {t('skills.meetingBots.recentCallDuration').replace('{seconds}', String(duration))}
          </span>
          {record.owner_display_name?.trim() && (
            <span>
              {t('skills.meetingBots.recentCallAddedBy').replace(
                '{name}',
                record.owner_display_name.trim()
              )}
            </span>
          )}
        </div>
        {participants.length > 0 && (
          <p className="text-[11px] text-content-muted">
            {participants.length === 1
              ? t('skills.meetingBots.history.participantCount').replace(
                  '{count}',
                  String(participants.length)
                )
              : t('skills.meetingBots.history.participantCountPlural').replace(
                  '{count}',
                  String(participants.length)
                )}
            {': '}
            {participants.join(', ')}
          </p>
        )}
      </div>

      {/* Detail body */}
      {(status === 'idle' || status === 'loading') && (
        <p className="text-[11px] text-content-faint">
          {t('skills.meetingBots.callDetailLoading')}
        </p>
      )}

      {status === 'error' && (
        <p className="text-[11px] text-coral-600 dark:text-coral-400">
          {t('skills.meetingBots.callDetailError')}{' '}
          <button
            type="button"
            onClick={() => void loadDetail(record.request_id)}
            className="underline underline-offset-2 hover:text-coral-700 dark:hover:text-coral-300">
            {t('skills.meetingBots.callDetailRetry')}
          </button>
        </p>
      )}

      {status === 'loaded' &&
        !hasSummaryDetail(detail) &&
        (detail?.transcript ?? []).length === 0 && (
          <p className="text-[11px] text-content-faint">
            {t('skills.meetingBots.callDetailEmpty')}
          </p>
        )}

      {status === 'loaded' &&
        (hasSummaryDetail(detail) || (detail?.transcript ?? []).length > 0) && (
          <div className="space-y-4">
            {hasSummaryDetail(detail) && detail?.summary && (
              <div className="space-y-2">
                {detail.summary.headline.trim() && (
                  <p className="text-[12px] text-content-secondary">{detail.summary.headline}</p>
                )}
                {detail.summary.key_points.length > 0 && (
                  <div>
                    <p className="text-[10px] font-medium text-content-muted">
                      {t('skills.meetingBots.callKeyPointsHeading')}
                    </p>
                    <ul className="mt-0.5 list-disc space-y-0.5 pl-4 text-[11px] text-content-secondary">
                      {detail.summary.key_points.map((point, i) => (
                        <li key={i}>{point}</li>
                      ))}
                    </ul>
                  </div>
                )}
                {detail.summary.action_items.length > 0 && (
                  <ActionItemChecklist items={detail.summary.action_items} />
                )}
              </div>
            )}
            {(detail?.transcript ?? []).length > 0 && (
              <TranscriptViewer lines={detail!.transcript} />
            )}
          </div>
        )}
    </div>
  );
}

export default HistoryDetail;
