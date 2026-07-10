/**
 * UpcomingTable — full-width table of upcoming calendar meetings with
 * conferencing links.
 *
 * Columns: WHEN / MEETING / PLATFORM / PEOPLE / JOIN POLICY / (action)
 *
 * Date-group separators: Today / Tomorrow / <date>
 *
 * Imminent meetings (≤ 5 min until start) get an accent row and a
 * "Join now" button. Other rows show a quieter join/open-link affordance.
 *
 * JOIN POLICY toggle: local state only (Phase 2). Phase 3 adds persistence.
 */
import debug from 'debug';
import { useState } from 'react';

import { useMascotManifest } from '../../features/human/Mascot/manifest/useMascotManifest';
import { useT } from '../../lib/i18n/I18nContext';
import {
  joinMeetViaBackendBot,
  type MeetingPlatform,
  setEventPolicy,
  type UpcomingMeeting,
} from '../../services/meetCallService';
import { selectBackendMeetStatus, selectBackendMeetUrl } from '../../store/backendMeetSlice';
import { useAppSelector } from '../../store/hooks';
import {
  selectCustomPrimaryColor,
  selectCustomSecondaryColor,
  selectDualMascotEnabled,
  selectMascotColor,
  selectMeetingMascotVoicePair,
  selectSelectedMascotId,
} from '../../store/mascotSlice';
import { selectPersonaDescription, selectPersonaDisplayName } from '../../store/personaSlice';
import Button from '../ui/Button';
import { type JoinPolicy, JoinPolicyToggle } from './JoinPolicyToggle';
import {
  buildMeetingMascots,
  inferPlatformFromUrl,
  platformLabel,
  platformLogoUrl,
  resolveMeetingBotMascotId,
} from './meetingUtils';
import { useUpcomingMeetings } from './useUpcomingMeetings';

const log = debug('meetings:upcoming-table');

const IMMINENT_THRESHOLD_MS = 5 * 60 * 1000; // 5 minutes

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function localDayKey(ms: number): string {
  const d = new Date(ms);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
}

function todayKey(): string {
  return localDayKey(Date.now());
}

function tomorrowKey(): string {
  return localDayKey(Date.now() + 86_400_000);
}

/**
 * Format a future time as a relative label ("in 5m", "in 2h") plus an
 * absolute time string (e.g. "14:30").
 *
 * All user-visible strings are routed through i18n. The caller must
 * supply the `t` function from `useT()`.
 */
function formatWhen(
  ms: number,
  t: (key: string) => string
): { relative: string; absolute: string } {
  const diffMs = ms - Date.now();
  const absolute = new Date(ms).toLocaleTimeString(undefined, {
    hour: '2-digit',
    minute: '2-digit',
  });

  if (diffMs < 0) {
    return { relative: t('skills.meetingBots.relative.now'), absolute };
  }
  const minutes = Math.floor(diffMs / 60_000);
  if (minutes < 60) {
    return {
      relative: t('skills.meetingBots.relative.inMinutes').replace('{count}', String(minutes)),
      absolute,
    };
  }
  const hours = Math.floor(minutes / 60);
  return {
    relative: t('skills.meetingBots.relative.inHours').replace('{count}', String(hours)),
    absolute,
  };
}

function isImminent(startTimeMs: number): boolean {
  return startTimeMs - Date.now() <= IMMINENT_THRESHOLD_MS;
}

// ---------------------------------------------------------------------------
// Group meetings by local date
// ---------------------------------------------------------------------------

interface DateGroup {
  label: string;
  meetings: UpcomingMeeting[];
}

function groupByDate(
  meetings: UpcomingMeeting[],
  todayLabel: string,
  tomorrowLabel: string
): DateGroup[] {
  const today = todayKey();
  const tomorrow = tomorrowKey();
  const buckets = new Map<string, { label: string; meetings: UpcomingMeeting[] }>();

  for (const m of meetings) {
    const key = localDayKey(m.start_time_ms);
    if (!buckets.has(key)) {
      let label: string;
      if (key === today) label = todayLabel;
      else if (key === tomorrow) label = tomorrowLabel;
      else {
        label = new Date(m.start_time_ms).toLocaleDateString(undefined, {
          weekday: 'short',
          month: 'short',
          day: 'numeric',
        });
      }
      buckets.set(key, { label, meetings: [] });
    }
    buckets.get(key)!.meetings.push(m);
  }

  return Array.from(buckets.values());
}

// ---------------------------------------------------------------------------
// Platform filter
// ---------------------------------------------------------------------------

type PlatformFilter = MeetingPlatform | 'all';

/**
 * Resolve the platform that a meeting row actually displays: the explicit
 * `platform` field first, then inferred from the conferencing URL.  This must
 * match exactly what `MeetingRow` renders so that filtering is consistent
 * with what the user sees.
 */
function effectivePlatform(m: UpcomingMeeting): MeetingPlatform | null {
  if (m.platform) return m.platform as MeetingPlatform;
  return m.meet_url ? inferPlatformFromUrl(m.meet_url) : null;
}

function filterMeetings(meetings: UpcomingMeeting[], filter: PlatformFilter): UpcomingMeeting[] {
  if (filter === 'all') return meetings;
  return meetings.filter(m => effectivePlatform(m) === filter);
}

// ---------------------------------------------------------------------------
// Row component
// ---------------------------------------------------------------------------

interface MeetingRowProps {
  meeting: UpcomingMeeting;
  joinPolicy: JoinPolicy;
  onJoinPolicyChange: (v: JoinPolicy) => void;
  onJoin: (m: UpcomingMeeting) => void;
  joining: boolean;
  joined: boolean;
}

function MeetingRow({
  meeting,
  joinPolicy,
  onJoinPolicyChange,
  onJoin,
  joining,
  joined,
}: MeetingRowProps) {
  const { t } = useT();
  const imminent = isImminent(meeting.start_time_ms);
  const { relative, absolute } = formatWhen(meeting.start_time_ms, t);

  const platform = (meeting.platform ??
    (meeting.meet_url
      ? (inferPlatformFromUrl(meeting.meet_url) ?? null)
      : null)) as MeetingPlatform | null;
  const logoUrl = platform ? platformLogoUrl(platform) : null;
  const platformName = platform ? platformLabel(platform, t) : '—';

  return (
    <tr
      className={[
        'border-b border-line/50 transition-colors',
        imminent ? 'bg-amber-500/5 hover:bg-amber-500/10' : 'hover:bg-surface-hover',
      ]
        .filter(Boolean)
        .join(' ')}>
      {/* WHEN */}
      <td className="py-2 px-3 whitespace-nowrap">
        <div className="flex flex-col">
          <span
            className={[
              'text-xs font-medium',
              imminent ? 'text-amber-400' : 'text-content-primary',
            ].join(' ')}>
            {relative}
          </span>
          <span className="text-xs text-content-secondary">{absolute}</span>
        </div>
      </td>

      {/* MEETING */}
      <td className="py-2 px-3 min-w-0 max-w-xs">
        <span
          className="block truncate text-sm text-content-primary font-medium"
          title={meeting.title}>
          {meeting.title}
        </span>
      </td>

      {/* PLATFORM */}
      <td className="py-2 px-3 whitespace-nowrap">
        <div className="flex items-center gap-1.5">
          {logoUrl && (
            <img
              src={logoUrl}
              alt=""
              aria-hidden="true"
              className="w-4 h-4 rounded-sm shrink-0"
              onError={e => {
                (e.currentTarget as HTMLImageElement).style.display = 'none';
              }}
            />
          )}
          <span className="text-xs text-content-secondary">{platformName}</span>
        </div>
      </td>

      {/* PEOPLE */}
      <td className="py-2 px-3 whitespace-nowrap text-xs text-content-secondary">
        {meeting.participant_count != null
          ? t('skills.meetingBots.upcoming.participants').replace(
              '{count}',
              String(meeting.participant_count)
            )
          : '—'}
      </td>

      {/* JOIN POLICY */}
      <td className="py-2 px-3">
        <div className="flex flex-col gap-0.5">
          <JoinPolicyToggle value={joinPolicy} onChange={onJoinPolicyChange} compact />
          {joinPolicy === 'auto' && (
            <span className="text-[10px] text-content-secondary/60 whitespace-nowrap">
              {t('skills.meetingBots.upcoming.autoJoinsAt').replace(
                '{time}',
                new Date(meeting.start_time_ms).toLocaleTimeString(undefined, {
                  hour: '2-digit',
                  minute: '2-digit',
                })
              )}
            </span>
          )}
          {joinPolicy === 'ask' && (
            <span className="text-[10px] text-content-secondary/60 whitespace-nowrap">
              {t('skills.meetingBots.upcoming.asksAtStart')}
            </span>
          )}
        </div>
      </td>

      {/* ACTION */}
      <td className="py-2 px-3 whitespace-nowrap">
        {joined ? (
          <span className="inline-flex items-center gap-1.5 text-xs font-medium text-emerald-400">
            <span
              className="w-1.5 h-1.5 rounded-full bg-emerald-400 animate-pulse"
              aria-hidden="true"
            />
            {t('skills.meetingBots.liveBadge')}
          </span>
        ) : imminent ? (
          <Button
            variant="primary"
            size="xs"
            disabled={joining || !meeting.meet_url}
            aria-label={t('skills.meetingBots.upcoming.joinNowAriaLabel').replace(
              '{title}',
              meeting.title
            )}
            onClick={() => onJoin(meeting)}>
            {joining ? '…' : t('skills.meetingBots.upcoming.joinNow')}
          </Button>
        ) : meeting.meet_url ? (
          <Button variant="tertiary" size="xs" disabled={joining} onClick={() => onJoin(meeting)}>
            {t('skills.meetingBots.upcoming.join')}
          </Button>
        ) : null}
      </td>
    </tr>
  );
}

// ---------------------------------------------------------------------------
// Loading skeleton
// ---------------------------------------------------------------------------

function SkeletonRow() {
  return (
    <tr className="border-b border-line/50 animate-pulse">
      {Array.from({ length: 6 }).map((_, i) => (
        <td key={i} className="py-2 px-3">
          <div className="h-4 bg-surface-hover rounded w-16" />
        </td>
      ))}
    </tr>
  );
}

// ---------------------------------------------------------------------------
// Main component
// ---------------------------------------------------------------------------

export interface UpcomingTableProps {
  lookaheadMinutes?: number;
  limit?: number;
  /**
   * Master calendar-watch switch value from meet settings.
   * `null` = unknown (fetch in progress / failed) — hint is suppressed.
   * `false` = off — show hint when there are upcoming meetings.
   * `true` = on — no hint needed.
   */
  watchCalendar?: boolean | null;
  /**
   * The user's saved meeting display name (from meet settings). Used as the
   * bot's reply anchor when joining via "Join now" — when set, the bot joins in
   * reply mode instead of listen-only so it can respond to the user.
   */
  replyDisplayName?: string;
}

export function UpcomingTable({
  lookaheadMinutes,
  limit,
  watchCalendar = null,
  replyDisplayName = '',
}: UpcomingTableProps) {
  const { t } = useT();
  const { meetings, loading, error, refresh } = useUpcomingMeetings(lookaheadMinutes, limit);

  const [platformFilter, setPlatformFilter] = useState<PlatformFilter>('all');
  const [joinPolicies, setJoinPolicies] = useState<Record<string, JoinPolicy>>({});
  const [joiningId, setJoiningId] = useState<string | null>(null);

  // Persona / mascot selectors for bot join params — mirrors MeetComposer.tsx.
  const personaDisplayName = useAppSelector(selectPersonaDisplayName);
  const personaDescription = useAppSelector(selectPersonaDescription);
  const selectedMascotId = useAppSelector(selectSelectedMascotId);
  const mascotColor = useAppSelector(selectMascotColor);
  const customPrimaryColor = useAppSelector(selectCustomPrimaryColor);
  const customSecondaryColor = useAppSelector(selectCustomSecondaryColor);
  // Dual-mascot config (issue #4277) — see MeetComposer for the rationale;
  // both join sites resolve the two-mascot slots the same way so a "Join now"
  // and a manual join behave identically.
  const dualMascotEnabled = useAppSelector(selectDualMascotEnabled);
  const mascotVoicePair = useAppSelector(selectMeetingMascotVoicePair);
  // Manifest drives name-addressed routing (#4277 follow-up): tag each dual slot
  // with its display name so "Hey Toshi …" routes to that mascot.
  const { manifest } = useMascotManifest();

  // Live in-call state — lets a row detect that its meeting is already joined
  // and suppress the "Join now" button. correlationId is a fresh per-join UUID
  // (#4338), so the backendMeet slice's meetingId never equals
  // calendar_event_id — match the joined meet_url instead.
  const backendMeetStatus = useAppSelector(selectBackendMeetStatus);
  const backendMeetUrl = useAppSelector(selectBackendMeetUrl);
  const isMeetingJoined = (m: UpcomingMeeting): boolean => {
    if (backendMeetStatus !== 'active' && backendMeetStatus !== 'joining') return false;
    return Boolean(backendMeetUrl && m.meet_url && backendMeetUrl === m.meet_url);
  };

  // Resolve bot join params the same way MeetComposer does — via
  // resolveMeetingBotMascotId, so a manifest-only id the backend bot doesn't
  // recognize is dropped here too (a raw `selectedMascotId` fallback would let
  // it through and diverge from the MeetComposer join).
  const mascotId = resolveMeetingBotMascotId(selectedMascotId, mascotColor);
  const riveColors =
    mascotColor === 'custom'
      ? { primaryColor: customPrimaryColor, secondaryColor: customSecondaryColor }
      : undefined;
  // Bot join name — hoisted to component-body scope because both the `mascots`
  // array here and the wake phrase inside handleJoin need it.
  const agentName = personaDisplayName.trim() || 'Tiny';
  // Two-mascot slots (issue #4277) — built via the shared helper so this
  // scheduled-join path and the MeetComposer live-join path stay behaviorally
  // identical.
  const mascots = buildMeetingMascots({
    dualMascotEnabled,
    mascotVoicePair,
    manifest,
    mascotId,
    riveColors,
    agentName,
  });

  const handleJoin = async (meeting: UpcomingMeeting) => {
    if (!meeting.meet_url) return;
    const platform = meeting.platform ?? inferPlatformFromUrl(meeting.meet_url) ?? undefined;
    // Reply anchor: the display name the user saved on the Meetings page. When
    // present the bot joins in reply mode (not listen-only) so it can respond to
    // the user; without it we keep the safe listen-only default (the bot has no
    // one to reply to and would otherwise talk to everyone).
    const anchor = replyDisplayName.trim();
    // Reply mode gates the bot behind a wake phrase so it only reacts when
    // addressed ("Hey Alex, …"), never to every caption from the anchor —
    // mirroring MeetComposer. The bot joins as `agentName` (hoisted above), so
    // the phrase must match it. Listen-only joins (no anchor) send no wake phrase.
    const wakePhrase = anchor ? `Hey ${agentName}` : undefined;
    // Mint a fresh correlation id per join. It becomes the call record's
    // `request_id` (recent-calls list key + per-call detail filename), so it
    // MUST be unique per join — reusing the deterministic `calendar_event_id`
    // collapsed re-joins of the same event onto one request_id, overwriting the
    // earlier call's transcript and double-highlighting the history row (#4338).
    // `calendar_event_id` stays the dedup/policy key only (handleJoinPolicyChange,
    // setJoiningId), mirroring the background auto-join in calendar.rs.
    const correlationId = crypto.randomUUID();
    log(
      '[upcoming] joining %s platform=%s reply_mode=%s correlationId=%s',
      meeting.calendar_event_id,
      platform,
      Boolean(anchor),
      correlationId
    );
    setJoiningId(meeting.calendar_event_id);
    // Name-addressing (#4277 follow-up) trace: mascot ids + names sent to the
    // backend. Empty `name` on a slot ⇒ name addressing can't route to it.
    log(
      '[upcoming] join mascots=%o wakePhrase=%s',
      mascots?.map(m => ({ mascotId: m.mascotId, name: m.name })),
      wakePhrase
    );
    try {
      await joinMeetViaBackendBot({
        meetUrl: meeting.meet_url,
        platform: platform as MeetingPlatform | undefined,
        agentName,
        systemPrompt: personaDescription || undefined,
        mascotId: mascotId || undefined,
        // Dual-mascot slots (issue #4277); undefined for single-mascot calls.
        mascots,
        respondToParticipant: anchor || undefined,
        wakePhrase,
        listenOnly: !anchor,
        correlationId,
        riveColors,
      });
    } catch (err) {
      log('[upcoming] join error: %s', err instanceof Error ? err.message : String(err));
    } finally {
      setJoiningId(null);
    }
  };

  const handleJoinPolicyChange = async (id: string, v: JoinPolicy) => {
    const prev =
      joinPolicies[id] ??
      (meetings.find(m => m.calendar_event_id === id)?.join_policy as JoinPolicy | undefined) ??
      'ask';
    // Optimistic update
    setJoinPolicies(current => ({ ...current, [id]: v }));
    log('[upcoming] set event policy id=%s policy=%s', id, v);
    try {
      await setEventPolicy(id, v);
    } catch (err) {
      // Revert on failure — but ONLY if the value still matches what THIS call
      // optimistically set. A newer concurrent call may have already moved the
      // value to something else; clobbering it would discard that change.
      log(
        '[upcoming] set event policy failed, reverting: %s',
        err instanceof Error ? err.message : String(err)
      );
      setJoinPolicies(curr => (curr[id] === v ? { ...curr, [id]: prev } : curr));
    }
  };

  const filtered = filterMeetings(meetings, platformFilter);
  const groups = groupByDate(
    filtered,
    t('skills.meetingBots.upcoming.today'),
    t('skills.meetingBots.upcoming.tomorrow')
  );

  // Collect unique effective platforms for the filter dropdown — uses the same
  // effectivePlatform() helper as filterMeetings() so the options are consistent
  // with what each row displays (including inferred-from-URL platforms).
  const presentPlatforms = Array.from(
    new Set(meetings.map(m => effectivePlatform(m)).filter((p): p is MeetingPlatform => p != null))
  );

  return (
    <div className="w-full rounded-2xl border border-line bg-surface shadow-soft overflow-hidden">
      {/* Table header bar */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-line/50">
        <h3 className="text-sm font-semibold text-content-primary">
          {t('skills.meetingBots.upcoming.heading')}
        </h3>

        <div className="flex items-center gap-2">
          {/* Platform filter */}
          {presentPlatforms.length > 1 && (
            <select
              className="text-xs bg-transparent text-content-secondary border border-line/50 rounded px-1.5 py-0.5 focus:outline-none"
              value={platformFilter}
              onChange={e => setPlatformFilter(e.target.value as PlatformFilter)}
              aria-label={t('skills.meetingBots.upcoming.filterAll')}>
              <option value="all">{t('skills.meetingBots.upcoming.filterAll')}</option>
              {presentPlatforms.map(p => (
                <option key={p} value={p}>
                  {platformLabel(p, t)}
                </option>
              ))}
            </select>
          )}

          {/* Refresh button */}
          <Button
            variant="tertiary"
            size="xs"
            iconOnly
            aria-label={t('skills.meetingBots.upcoming.refresh')}
            onClick={refresh}
            disabled={loading}>
            {/* Minimal SVG refresh icon */}
            <svg width="12" height="12" viewBox="0 0 12 12" fill="none" aria-hidden="true">
              <path
                d="M10 6a4 4 0 1 1-1.17-2.83M10 2v3H7"
                stroke="currentColor"
                strokeWidth="1.5"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          </Button>
        </div>
      </div>

      {/* Watch-calendar off hint — only when there are meetings and watchCalendar is explicitly false */}
      {meetings.length > 0 && watchCalendar === false && (
        <div
          role="note"
          className="flex items-start gap-2 px-3 py-2 bg-amber-500/10 border-b border-amber-500/20 text-xs text-amber-600 dark:text-amber-400">
          <svg
            width="14"
            height="14"
            viewBox="0 0 16 16"
            fill="none"
            aria-hidden="true"
            className="mt-0.5 shrink-0">
            <path
              d="M8 2a6 6 0 1 0 0 12A6 6 0 0 0 8 2Zm0 3.5v3m0 2h.01"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
          <span>{t('skills.meetingBots.upcoming.watchCalendarHint')}</span>
        </div>
      )}

      {/* Table */}
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead>
            <tr className="text-xs text-content-secondary border-b border-line/50">
              <th className="py-1.5 px-3 text-left font-medium">
                {t('skills.meetingBots.upcoming.when')}
              </th>
              <th className="py-1.5 px-3 text-left font-medium">
                {t('skills.meetingBots.upcoming.meeting')}
              </th>
              <th className="py-1.5 px-3 text-left font-medium">
                {t('skills.meetingBots.upcoming.platform')}
              </th>
              <th className="py-1.5 px-3 text-left font-medium">
                {t('skills.meetingBots.upcoming.people')}
              </th>
              <th className="py-1.5 px-3 text-left font-medium">
                {t('skills.meetingBots.upcoming.joinPolicy')}
              </th>
              <th className="py-1.5 px-3 text-left font-medium" />
            </tr>
          </thead>
          <tbody>
            {loading && meetings.length === 0 && (
              <>
                <SkeletonRow />
                <SkeletonRow />
                <SkeletonRow />
              </>
            )}

            {!loading && error && (
              <tr>
                <td colSpan={6} className="py-6 px-3 text-center">
                  <p className="text-sm text-coral-400 mb-2">
                    {t('skills.meetingBots.upcoming.error')}
                  </p>
                  <Button variant="secondary" size="xs" onClick={refresh}>
                    {t('skills.meetingBots.upcoming.retry')}
                  </Button>
                </td>
              </tr>
            )}

            {!loading && !error && filtered.length === 0 && (
              <tr>
                <td colSpan={6} className="py-8 px-3 text-center">
                  <p className="text-sm text-content-secondary">
                    {t('skills.meetingBots.upcoming.empty')}
                  </p>
                </td>
              </tr>
            )}

            {groups.flatMap(group => [
              /* Date-group separator row */
              <tr key={`gh-${group.label}`}>
                <td
                  colSpan={6}
                  className="py-1 px-3 text-xs font-semibold text-content-secondary uppercase tracking-wide bg-surface-hover/50 border-b border-line/30">
                  {group.label}
                </td>
              </tr>,
              /* Meeting rows for this group */
              ...group.meetings.map(m => {
                const policy: JoinPolicy =
                  (joinPolicies[m.calendar_event_id] as JoinPolicy | undefined) ??
                  (m.join_policy as JoinPolicy) ??
                  'ask';
                return (
                  <MeetingRow
                    key={m.calendar_event_id}
                    meeting={m}
                    joinPolicy={policy}
                    onJoinPolicyChange={v => handleJoinPolicyChange(m.calendar_event_id, v)}
                    onJoin={handleJoin}
                    joining={joiningId === m.calendar_event_id}
                    joined={isMeetingJoined(m)}
                  />
                );
              }),
            ])}
          </tbody>
        </table>
      </div>
    </div>
  );
}
