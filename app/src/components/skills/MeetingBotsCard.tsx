// Meeting bots entry point on the Skills "Integrations" section.
//
// Surfaces as a compact banner: clicking opens a modal that asks the
// backend to send a Recall.ai-hosted mascot bot into the meeting. The
// backend streams replies, harness requests, and the final transcript
// back through the core Socket.IO bridge.

import { useCallback, useEffect, useMemo, useState } from 'react';

import { RiveMascot, type MascotFace } from '../../features/human/Mascot';
import { useT } from '../../lib/i18n/I18nContext';
import {
  joinMeetViaBackendBot,
  leaveBackendMeetBot,
  listMeetCalls,
  type MascotMeetPlatform,
  type MeetCallRecord,
} from '../../services/meetCallService';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
import {
  type BackendMeetHarnessEvent,
  type BackendMeetReplyEvent,
  type BackendMeetStatus,
  resetBackendMeet,
  selectBackendMeetLastHarness,
  selectBackendMeetLastReply,
  selectBackendMeetStatus,
  selectBackendMeetUrl,
  setBackendMeetJoining,
} from '../../store/backendMeetSlice';

type Toast = { type: 'success' | 'error' | 'info'; title: string; message?: string };

interface Props {
  onToast?: (toast: Toast) => void;
}

interface PlatformDef {
  platform: MascotMeetPlatform;
  labelKey: string;
  domainHintKey: string;
  comingSoon?: boolean;
}

const PLATFORMS: PlatformDef[] = [
  {
    platform: 'gmeet',
    labelKey: 'skills.meetingBots.platforms.gmeet',
    domainHintKey: 'skills.meetingBots.platformHints.gmeet',
  },
  {
    platform: 'zoom',
    labelKey: 'skills.meetingBots.platforms.zoom',
    domainHintKey: 'skills.meetingBots.platformHints.zoom',
  },
  {
    platform: 'teams',
    labelKey: 'skills.meetingBots.platforms.teams',
    domainHintKey: 'skills.meetingBots.platformHints.teams',
  },
];

export default function MeetingBotsCard({ onToast }: Props) {
  const [open, setOpen] = useState(false);
  const status = useAppSelector(selectBackendMeetStatus);

  const showActive = status === 'active' || status === 'joining';

  return (
    <>
      {showActive ? (
        <ActiveMeetingView onToast={onToast} />
      ) : (
        <MeetingBotsBanner onClick={() => setOpen(true)} />
      )}
      {open && <MeetingBotsModal onClose={() => setOpen(false)} onToast={onToast} />}
    </>
  );
}

function faceFromMeetState(
  status: BackendMeetStatus,
  lastReply: BackendMeetReplyEvent | null,
  lastHarness: BackendMeetHarnessEvent | null,
): MascotFace {
  if (status === 'joining') return 'thinking';
  if (status === 'error') return 'concerned';
  if (status === 'ended') return 'happy';
  if (lastHarness) return 'thinking';
  if (lastReply) {
    const e = (lastReply.emotion ?? '').toLowerCase();
    if (e.includes('happy') || e.includes('pleased') || e.includes('joy') || e.includes('excit')) return 'happy';
    if (e.includes('celebrat') || e.includes('proud')) return 'celebrating';
    if (e.includes('concern') || e.includes('worried') || e.includes('unsure')) return 'concerned';
    if (e.includes('curious') || e.includes('interest')) return 'curious';
  }
  return 'idle';
}

function ActiveMeetingView({ onToast }: Props) {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const status = useAppSelector(selectBackendMeetStatus);
  const meetUrl = useAppSelector(selectBackendMeetUrl);
  const lastReply = useAppSelector(selectBackendMeetLastReply);
  const lastHarness = useAppSelector(selectBackendMeetLastHarness);
  const face = faceFromMeetState(status, lastReply, lastHarness);
  const meetingCode = useMemo(() => {
    if (!meetUrl) return '';
    try {
      const tail = new URL(meetUrl).pathname.replace(/^\/+/, '');
      return tail || meetUrl;
    } catch { return meetUrl; }
  }, [meetUrl]);

  const [leaving, setLeaving] = useState(false);

  const handleLeave = async () => {
    if (leaving) return;
    setLeaving(true);
    try {
      await leaveBackendMeetBot('user-requested');
    } catch (err) {
      onToast?.({ type: 'error', title: t('skills.meetingBots.couldNotStartTitle'), message: String(err) });
    } finally {
      setLeaving(false);
    }
  };

  const statusText = {
    joining: t('skills.meetingBots.liveStatusJoining'),
    active: t('skills.meetingBots.liveStatusActive'),
    ended: t('skills.meetingBots.liveStatusEnded'),
    error: t('skills.meetingBots.liveStatusError'),
    idle: '',
  }[status] ?? '';

  const canLeave = status === 'active' || status === 'joining';
  const isDone = status === 'ended' || status === 'error';

  return (
    <div className="relative overflow-hidden rounded-2xl border border-primary-200/60 dark:border-primary-500/30 bg-gradient-to-br from-primary-50 via-white to-amber-50 dark:from-primary-500/15 dark:via-neutral-900 dark:to-amber-500/10 p-4 shadow-soft animate-fade-up">
      <div className="flex items-center justify-between mb-3">
        <span className="flex items-center gap-1.5 rounded-full bg-coral-500/10 dark:bg-coral-400/15 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-coral-600 dark:text-coral-400">
          <span className="h-1.5 w-1.5 rounded-full bg-coral-500 animate-pulse" aria-hidden="true" />
          {t('skills.meetingBots.liveBadge')}
        </span>
        {canLeave && (
          <button type="button" onClick={handleLeave} disabled={leaving}
            className="rounded-xl px-3 py-1.5 text-xs font-medium bg-stone-100 dark:bg-neutral-800 text-stone-700 dark:text-neutral-300 hover:bg-stone-200 dark:hover:bg-neutral-700 disabled:opacity-50 disabled:cursor-not-allowed">
            {t('skills.meetingBots.leaveButton')}
          </button>
        )}
        {isDone && (
          <button type="button" onClick={() => dispatch(resetBackendMeet())}
            className="rounded-xl px-3 py-1.5 text-xs font-medium bg-stone-100 dark:bg-neutral-800 text-stone-700 dark:text-neutral-300 hover:bg-stone-200 dark:hover:bg-neutral-700">
            {t('common.close')}
          </button>
        )}
      </div>
      <div className="flex items-center gap-4">
        <div className="w-20 h-20 flex-shrink-0">
          <RiveMascot face={face} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
            {t('skills.meetingBots.liveTitle')}
          </div>
          <div className="mt-0.5 text-xs text-stone-500 dark:text-neutral-400">{statusText}</div>
          {meetingCode && (
            <div className="mt-1 truncate font-mono text-[11px] text-stone-600 dark:text-neutral-400">{meetingCode}</div>
          )}
          {lastReply?.reply && (
            <div className="mt-1.5 text-xs text-stone-600 dark:text-neutral-300 line-clamp-2 italic">
              &ldquo;{lastReply.reply}&rdquo;
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function MeetingBotsBanner({ onClick }: { onClick: () => void }) {
  const { t } = useT();
  return (
    <button
      type="button"
      onClick={onClick}
      data-testid="meeting-bots-banner"
      className="group relative w-full overflow-hidden rounded-2xl border border-primary-200/60 dark:border-primary-500/30 bg-gradient-to-br from-primary-50 via-white to-amber-50 dark:from-primary-500/15 dark:via-neutral-900 dark:to-amber-500/10 p-4 text-left shadow-soft transition hover:-translate-y-0.5 hover:shadow-md focus:outline-none focus-visible:ring-2 focus-visible:ring-primary-400 animate-fade-up">
      {/* Decorative gradient orbs — purely cosmetic, hidden from a11y. */}
      <span
        aria-hidden="true"
        className="pointer-events-none absolute -right-8 -top-8 h-32 w-32 rounded-full bg-primary-300/30 blur-2xl transition group-hover:bg-primary-300/40"
      />
      <span
        aria-hidden="true"
        className="pointer-events-none absolute -bottom-10 -left-6 h-24 w-24 rounded-full bg-amber-300/30 blur-2xl"
      />

      <div className="relative flex items-center gap-3">
        <span
          aria-hidden="true"
          className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-white dark:bg-neutral-900 text-base font-bold text-primary-600 shadow-soft ring-1 ring-primary-200/70">
          {/* Tiny "wave" mark — three dots that animate on hover. */}
          <span className="flex items-end gap-0.5">
            <span className="h-2 w-1 rounded-full bg-primary-500 transition group-hover:h-3" />
            <span className="h-3 w-1 rounded-full bg-primary-500 transition group-hover:h-4" />
            <span className="h-2 w-1 rounded-full bg-primary-500 transition group-hover:h-3" />
          </span>
        </span>

        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h2 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
              {t('skills.meetingBots.bannerTitle')}
            </h2>
            <span className="rounded-full bg-primary-100 dark:bg-primary-500/20 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-primary-700 dark:text-primary-300">
              {t('skills.meetingBots.newBadge')}
            </span>
          </div>
          <p className="mt-0.5 line-clamp-1 text-[11px] leading-relaxed text-stone-600 dark:text-neutral-300">
            {t('skills.meetingBots.bannerDesc')}
          </p>
        </div>

        <span
          aria-hidden="true"
          className="ml-2 hidden text-stone-400 dark:text-neutral-500 transition group-hover:text-stone-600 dark:group-hover:text-neutral-300 sm:inline">
          →
        </span>
      </div>
    </button>
  );
}

interface ModalProps {
  onClose: () => void;
  onToast?: (toast: Toast) => void;
}

export function MeetingBotsModal({ onClose, onToast }: ModalProps) {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const [platform, setPlatform] = useState<MascotMeetPlatform>('gmeet');
  const [meetUrl, setMeetUrl] = useState('');
  const [displayName, setDisplayName] = useState('OpenHuman');
  const [respondToParticipant, setRespondToParticipant] = useState('');
  const [wakePhrase, setWakePhrase] = useState('Hey OpenHuman');
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Recent-calls history loaded from core when the modal opens.
  // `null` means "not yet fetched"; `[]` means "fetched, no rows".
  // Separating the two lets the UI render a "Loading…" hint on
  // first open without flashing a misleading empty state.
  const [recentCalls, setRecentCalls] = useState<MeetCallRecord[] | null>(null);
  const [recentError, setRecentError] = useState<string | null>(null);

  const refreshRecentCalls = useCallback(async () => {
    setRecentError(null);
    try {
      const rows = await listMeetCalls(20);
      setRecentCalls(rows);
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to load recent calls.';
      console.warn('[meeting-bots] listMeetCalls failed:', err);
      setRecentError(message);
      setRecentCalls([]);
    }
  }, []);

  useEffect(() => {
    // Fire-and-forget on mount; the modal is short-lived (closes on
    // submit or Cancel) so a slow RPC here can't pile up.
    void refreshRecentCalls();
  }, [refreshRecentCalls]);

  const selected = PLATFORMS.find(p => p.platform === platform) ?? PLATFORMS[0];
  const selectedLabel = t(selected.labelKey);
  const isComingSoon = !!selected.comingSoon;

  // Esc closes the modal — matches the OpenhumanLinkModal pattern.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  const handleSubmit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setError(null);
    if (isComingSoon) {
      setError(t('skills.meetingBots.platformComingSoon').replace('{label}', selectedLabel));
      return;
    }
    setSubmitting(true);
    try {
      // Optimistically update Redux state so the banner transitions to
      // the ActiveMeetingView immediately, before the backend responds.
      dispatch(setBackendMeetJoining({ meetUrl: meetUrl.trim() }));
      // Backend Recall.ai bot: sends the mascot into the meeting via
      // the backend's Recall.ai integration. The backend joins as a
      // participant, renders the mascot as the bot's camera feed, and
      // streams transcript events back over Socket.IO.
      await joinMeetViaBackendBot({
        meetUrl,
        displayName,
        platform,
        agentName: displayName,
        respondToParticipant: respondToParticipant.trim() || undefined,
        wakePhrase: wakePhrase.trim() || undefined,
      });
      onToast?.({
        type: 'success',
        title: t('skills.meetingBots.joiningTitle'),
        message: t('skills.meetingBots.joiningMessage'),
      });
      setMeetUrl('');
      onClose();
    } catch (err) {
      const message = err instanceof Error ? err.message : t('skills.meetingBots.failedToStart');
      setError(message);
      onToast?.({ type: 'error', title: t('skills.meetingBots.couldNotStartTitle'), message });
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={t('skills.meetingBots.modalAriaLabel')}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4"
      onClick={onClose}>
      <div
        className="w-full max-w-md overflow-hidden rounded-2xl bg-white dark:bg-neutral-900 shadow-xl"
        onClick={e => e.stopPropagation()}>
        {/* Header band — same fun gradient as the banner so the modal feels like
            a continuation of the click, not a context switch. */}
        <div className="relative bg-gradient-to-br from-primary-50 via-white to-amber-50 dark:from-primary-500/15 dark:via-neutral-900 dark:to-amber-500/10 px-5 py-4">
          <button
            type="button"
            onClick={onClose}
            aria-label={t('common.close')}
            className="absolute right-3 top-3 rounded-full p-1 text-stone-500 dark:text-neutral-400 hover:bg-white/80 dark:hover:bg-neutral-800/60 hover:text-stone-800 dark:hover:text-neutral-100">
            ✕
          </button>
          <h2 className="text-base font-semibold text-stone-900 dark:text-neutral-100">{t('skills.meetingBots.modalTitle')}</h2>
          <p className="mt-1 text-xs leading-relaxed text-stone-600 dark:text-neutral-300">
            {t('skills.meetingBots.modalDesc')}
          </p>
        </div>

        <div className="space-y-4 p-5">
          <div className="flex flex-wrap gap-1.5">
            {PLATFORMS.map(p => {
              const active = p.platform === platform;
              return (
                <button
                  key={p.platform}
                  type="button"
                  onClick={() => {
                    setPlatform(p.platform);
                    setError(null);
                  }}
                  className={`rounded-full px-3 py-1 text-[11px] font-medium transition ${
                    active
                      ? 'bg-primary-500 text-white'
                      : 'bg-stone-100 dark:bg-neutral-800 text-stone-600 dark:text-neutral-300 hover:bg-stone-200 dark:hover:bg-neutral-700'
                  }`}>
                  {t(p.labelKey)}
                  {p.comingSoon && (
                    <span className="ml-1 opacity-70">
                      · {t('skills.meetingBots.soonSuffix')}
                    </span>
                  )}
                </button>
              );
            })}
          </div>

          <form onSubmit={handleSubmit} className="space-y-3">
            <label className="block">
              <span className="text-[10px] font-medium uppercase tracking-wide text-stone-500 dark:text-neutral-400">
                {t('skills.meetingBots.meetingLink')}
              </span>
              <input
                type="url"
                inputMode="url"
                autoComplete="off"
                spellCheck={false}
                value={meetUrl}
                onChange={e => setMeetUrl(e.target.value)}
                placeholder={t(selected.domainHintKey)}
                disabled={isComingSoon || submitting}
                autoFocus
                className="mt-1 w-full rounded-xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-100 disabled:cursor-not-allowed disabled:bg-stone-50 dark:disabled:bg-neutral-800/60"
                required
              />
            </label>

            <label className="block">
              <span className="text-[10px] font-medium uppercase tracking-wide text-stone-500 dark:text-neutral-400">
                {t('skills.meetingBots.displayName')}
              </span>
              <input
                type="text"
                value={displayName}
                onChange={e => setDisplayName(e.target.value)}
                maxLength={64}
                disabled={isComingSoon || submitting}
                className="mt-1 w-full rounded-xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-100 disabled:cursor-not-allowed disabled:bg-stone-50 dark:disabled:bg-neutral-800/60"
              />
            </label>

            <label className="block">
              <span className="text-[10px] font-medium uppercase tracking-wide text-stone-500 dark:text-neutral-400">
                {t('skills.meetingBots.respondToParticipant')}
              </span>
              <input
                type="text"
                value={respondToParticipant}
                onChange={e => setRespondToParticipant(e.target.value)}
                placeholder={t('skills.meetingBots.respondToParticipantHint')}
                maxLength={128}
                disabled={isComingSoon || submitting}
                className="mt-1 w-full rounded-xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-100 disabled:cursor-not-allowed disabled:bg-stone-50 dark:disabled:bg-neutral-800/60"
              />
              <span className="mt-1 block text-[10px] text-stone-400 dark:text-neutral-500">
                {t('skills.meetingBots.respondToParticipantDesc')}
              </span>
            </label>

            <label className="block">
              <span className="text-[10px] font-medium uppercase tracking-wide text-stone-500 dark:text-neutral-400">
                {t('skills.meetingBots.wakePhrase')}
              </span>
              <input
                type="text"
                value={wakePhrase}
                onChange={e => setWakePhrase(e.target.value)}
                placeholder={t('skills.meetingBots.wakePhraseHint')}
                maxLength={128}
                disabled={isComingSoon || submitting}
                className="mt-1 w-full rounded-xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-100 disabled:cursor-not-allowed disabled:bg-stone-50 dark:disabled:bg-neutral-800/60"
              />
              <span className="mt-1 block text-[10px] text-stone-400 dark:text-neutral-500">
                {t('skills.meetingBots.wakePhraseDesc')}
              </span>
            </label>

            {error && (
              <div
                role="alert"
                className="rounded-xl border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-3 py-2 text-xs text-coral-700 dark:text-coral-300">
                {error}
              </div>
            )}

            <div className="flex items-center justify-end gap-2 pt-1">
              <button
                type="button"
                onClick={onClose}
                className="rounded-xl px-3 py-2 text-sm font-medium text-stone-600 dark:text-neutral-300 hover:bg-stone-100 dark:hover:bg-neutral-800">
                {t('common.cancel')}
              </button>
              <button
                type="submit"
                disabled={
                  submitting || isComingSoon || !meetUrl.trim()
                }
                className="rounded-xl bg-primary-500 px-4 py-2 text-sm font-semibold text-white hover:bg-primary-600 disabled:cursor-not-allowed disabled:bg-stone-200 dark:disabled:bg-neutral-700 disabled:text-stone-400 dark:disabled:text-neutral-500">
                {isComingSoon
                  ? t('skills.meetingBots.comingSoon').replace('{label}', selectedLabel)
                  : submitting
                    ? t('skills.meetingBots.starting')
                    : t('skills.meetingBots.sendTo').replace('{label}', selectedLabel)}
              </button>
            </div>
          </form>

          <RecentCallsSection rows={recentCalls} error={recentError} />
        </div>
      </div>
    </div>
  );
}

/**
 * Recent calls list rendered below the join form inside the same
 * modal — same surface where the user launches a call, so they see
 * their history without navigating away. Three states:
 *   - `rows === null`     → still loading (small spinner-y hint).
 *   - `rows === []`       → no calls yet (gentle empty state).
 *   - `rows.length > 0`   → render a compact list, newest first.
 *
 * `error` is shown inline above the list when the fetch failed but
 * doesn't block the form — the join path is independent.
 */
function RecentCallsSection({
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

      {error && (
        // Plain status text rather than role="alert" — the join form
        // already owns the alert role for the modal's primary error
        // surface. A failure to fetch history is informational, not
        // actionable, and shouldn't collide with the form's a11y
        // announcement.
        <p className="mt-2 text-[11px] text-coral-600 dark:text-coral-400">{error}</p>
      )}

      {rows === null ? (
        <p className="mt-2 text-[11px] text-stone-400 dark:text-neutral-500">{t('skills.meetingBots.recentCallsLoading')}</p>
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
  // Show the trailing meeting code (`abc-defg-hij`) rather than the
  // full URL — the URL prefix is always `https://meet.google.com/`
  // and would just waste row width.
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
  return (
    <li className="rounded-lg px-2 py-1.5 text-[11px] text-stone-700 dark:text-neutral-300 hover:bg-stone-50 dark:hover:bg-neutral-800/40">
      <div className="flex items-center justify-between gap-2">
        <span className="truncate font-mono text-stone-800 dark:text-neutral-200">{meetingCode}</span>
        <span className="shrink-0 text-stone-400 dark:text-neutral-500">
          {formatRelativeTime(call.started_at_ms)}
        </span>
      </div>
      <div className="mt-0.5 flex items-center gap-3 text-[10px] text-stone-500 dark:text-neutral-400">
        <span>{call.turn_count} turn{call.turn_count === 1 ? '' : 's'}</span>
        <span>{duration}s on call</span>
      </div>
    </li>
  );
}

/**
 * Compact "12 min ago" / "yesterday" / "May 14" style stamp. Browser
 * `Intl.RelativeTimeFormat` would be nicer but pulls a much larger
 * locale data path; the targets here are short labels in a single
 * surface, not a full i18n investment.
 */
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
