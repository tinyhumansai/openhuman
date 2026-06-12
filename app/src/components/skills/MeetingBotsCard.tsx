// Meeting bots entry point on the Skills "Integrations" section.
//
// Surfaces as a compact banner: clicking opens a modal that asks the
// backend to send a Recall.ai-hosted mascot bot into the meeting. The
// backend streams replies, harness requests, and the final transcript
// back through the core Socket.IO bridge.
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { type MascotFace, RiveMascot } from '../../features/human/Mascot';
import { useT } from '../../lib/i18n/I18nContext';
import {
  joinMeetViaBackendBot,
  leaveBackendMeetBot,
  listMeetCalls,
  type MeetCallRecord,
} from '../../services/meetCallService';
import {
  type BackendMeetHarnessEvent,
  type BackendMeetReplyEvent,
  type BackendMeetStatus,
  resetBackendMeet,
  selectBackendMeetError,
  selectBackendMeetLastHarness,
  selectBackendMeetLastReply,
  selectBackendMeetListenOnly,
  selectBackendMeetStatus,
  selectBackendMeetUrl,
  setBackendMeetJoining,
} from '../../store/backendMeetSlice';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
import {
  selectCustomPrimaryColor,
  selectCustomSecondaryColor,
  selectMascotColor,
  selectSelectedMascotId,
} from '../../store/mascotSlice';
import { selectPersonaDescription, selectPersonaDisplayName } from '../../store/personaSlice';

type Toast = { type: 'success' | 'error' | 'info'; title: string; message?: string };

interface Props {
  onToast?: (toast: Toast) => void;
}

export default function MeetingBotsCard({ onToast }: Props) {
  const [open, setOpen] = useState(false);
  const status = useAppSelector(selectBackendMeetStatus);

  // Only switch to ActiveMeetingView once the backend has actually admitted
  // the bot. The 'joining' state still leaves the user on the modal so a
  // synchronous backend rejection (e.g. paid-plan gate) surfaces in the
  // modal's error alert instead of flashing through ActiveMeetingView.
  const showActive = status === 'active';

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
  lastHarness: BackendMeetHarnessEvent | null
): MascotFace {
  if (status === 'joining') return 'thinking';
  if (status === 'error') return 'concerned';
  if (status === 'ended') return 'happy';
  if (lastHarness) return 'thinking';
  if (lastReply) {
    const e = (lastReply.emotion ?? '').toLowerCase();
    if (e.includes('happy') || e.includes('pleased') || e.includes('joy') || e.includes('excit'))
      return 'happy';
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
  const listenOnly = useAppSelector(selectBackendMeetListenOnly);
  const lastReply = useAppSelector(selectBackendMeetLastReply);
  const lastHarness = useAppSelector(selectBackendMeetLastHarness);
  const face = faceFromMeetState(status, lastReply, lastHarness);
  const meetingCode = useMemo(() => {
    if (!meetUrl) return '';
    try {
      const tail = new URL(meetUrl).pathname.replace(/^\/+/, '');
      return tail || meetUrl;
    } catch {
      return meetUrl;
    }
  }, [meetUrl]);

  const [leaving, setLeaving] = useState(false);

  const handleLeave = async () => {
    if (leaving) return;
    setLeaving(true);
    try {
      await leaveBackendMeetBot('user-requested');
    } catch (err) {
      onToast?.({
        type: 'error',
        title: t('skills.meetingBots.couldNotStartTitle'),
        message: String(err),
      });
    } finally {
      setLeaving(false);
    }
  };

  const statusText = (() => {
    const base: Record<string, string> = {
      joining: t('skills.meetingBots.liveStatusJoining'),
      active: listenOnly
        ? t('skills.meetingBots.liveStatusListening')
        : t('skills.meetingBots.liveStatusActive'),
      ended: t('skills.meetingBots.liveStatusEnded'),
      error: t('skills.meetingBots.liveStatusError'),
      idle: '',
    };
    return base[status] ?? '';
  })();

  const canLeave = status === 'active' || status === 'joining';
  const isDone = status === 'ended' || status === 'error';

  return (
    <div className="relative overflow-hidden rounded-2xl border border-primary-200/60 dark:border-primary-500/30 bg-gradient-to-br from-primary-50 via-white to-amber-50 dark:from-primary-500/15 dark:via-neutral-900 dark:to-amber-500/10 p-4 shadow-soft animate-fade-up">
      <div className="flex items-center justify-between mb-3">
        <span className="flex items-center gap-1.5 rounded-full bg-coral-500/10 dark:bg-coral-400/15 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-coral-600 dark:text-coral-400">
          <span
            className="h-1.5 w-1.5 rounded-full bg-coral-500 animate-pulse"
            aria-hidden="true"
          />
          {t('skills.meetingBots.liveBadge')}
        </span>
        {canLeave && (
          <button
            type="button"
            onClick={handleLeave}
            disabled={leaving}
            className="rounded-xl px-3 py-1.5 text-xs font-medium bg-stone-100 dark:bg-neutral-800 text-stone-700 dark:text-neutral-300 hover:bg-stone-200 dark:hover:bg-neutral-700 disabled:opacity-50 disabled:cursor-not-allowed">
            {t('skills.meetingBots.leaveButton')}
          </button>
        )}
        {isDone && (
          <button
            type="button"
            onClick={() => dispatch(resetBackendMeet())}
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
            <div className="mt-1 truncate font-mono text-[11px] text-stone-600 dark:text-neutral-400">
              {meetingCode}
            </div>
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
  const [meetUrl, setMeetUrl] = useState('');
  // PASSIVE MODE: the respondTo state is retained for the
  // joinMeetViaBackendBot payload (always passed undefined now since the
  // backend ignores it) but the setter is unused while the input field
  // is hidden. Restore `[respondTo, setRespondTo]` if the field returns.
  const [respondTo] = useState('');
  const personaDisplayName = useAppSelector(selectPersonaDisplayName);
  const personaDescription = useAppSelector(selectPersonaDescription);
  const selectedMascotId = useAppSelector(selectSelectedMascotId);
  const mascotColor = useAppSelector(selectMascotColor);
  const customPrimaryColor = useAppSelector(selectCustomPrimaryColor);
  const customSecondaryColor = useAppSelector(selectCustomSecondaryColor);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const meetStatus = useAppSelector(selectBackendMeetStatus);
  const meetError = useAppSelector(selectBackendMeetError);
  // True once the user has clicked Join in this modal session — guards the
  // status-watching effect against stale redux state from a prior attempt.
  // Held as a ref (not state) so toggling it inside the effect doesn't
  // trigger a re-render cascade — see react-hooks/set-state-in-effect.
  const hasSubmittedRef = useRef(false);
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

  const selectedLabel = t('skills.meetingBots.platforms.gmeet');
  const agentName = personaDisplayName.trim() || 'OpenHuman';
  const systemPrompt = personaDescription.trim() || undefined;
  const mascotId = selectedMascotId ?? (mascotColor === 'custom' ? undefined : mascotColor);
  const riveColors =
    mascotColor === 'custom'
      ? { primaryColor: customPrimaryColor, secondaryColor: customSecondaryColor }
      : undefined;

  // The modal blocks dismissal while a join request is in flight so the
  // backend's admit/reject verdict (success toast + close, or inline
  // error) isn't skipped by an early Escape / backdrop click / X / Cancel.
  const canDismiss = !submitting;

  // Esc closes the modal — matches the OpenhumanLinkModal pattern.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && canDismiss) onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [canDismiss, onClose]);

  // After Join is clicked, watch the backend meet status. The RPC returns
  // as soon as the join request reaches the core, but the actual admit /
  // reject from the bot service arrives asynchronously over the socket.
  //  - 'active' → bot was admitted; surface success toast and close.
  //  - 'error'  → bot was rejected (paid-plan gate, capacity, etc); leave
  //               the modal open with the backend's message in the alert
  //               so the user is blocked from joining and sees why.
  useEffect(() => {
    if (!hasSubmittedRef.current) return;
    if (meetStatus === 'active') {
      hasSubmittedRef.current = false;
      onToast?.({
        type: 'success',
        title: t('skills.meetingBots.joiningTitle'),
        message: t('skills.meetingBots.joiningMessage'),
      });
      setMeetUrl('');
      onClose();
      return;
    }
    if (meetStatus === 'error') {
      hasSubmittedRef.current = false;
      const message = meetError?.trim() || t('skills.meetingBots.failedToStart');
      setError(message);
      setSubmitting(false);
      onToast?.({ type: 'error', title: t('skills.meetingBots.couldNotStartTitle'), message });
    }
  }, [meetStatus, meetError, onClose, onToast, t]);

  const handleSubmit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setError(null);
    setSubmitting(true);
    hasSubmittedRef.current = true;
    try {
      // Generate a correlation ID so every backend event for this session
      // can be tied back to this meeting.
      const meetingId = crypto.randomUUID();
      // Mark the meet slice as "joining" so the rest of the app reflects
      // the in-flight state. The modal stays open until the backend either
      // admits the bot (status → 'active', useEffect closes the modal) or
      // rejects it (status → 'error', useEffect surfaces the message).
      dispatch(setBackendMeetJoining({ meetUrl: meetUrl.trim(), meetingId }));
      // Backend Recall.ai bot: sends the mascot into the meeting via
      // the backend's Recall.ai integration. The backend joins as a
      // participant, renders the mascot as the bot's camera feed, and
      // streams transcript events back over Socket.IO.
      await joinMeetViaBackendBot({
        meetUrl,
        displayName: agentName,
        platform: 'gmeet',
        agentName,
        systemPrompt,
        mascotId,
        riveColors,
        correlationId: meetingId,
        respondToParticipant: respondTo.trim() || undefined,
      });
      // Don't close the modal here — wait for status === 'active' or 'error'
      // via the watcher useEffect above.
    } catch (err) {
      const message = err instanceof Error ? err.message : t('skills.meetingBots.failedToStart');
      setError(message);
      setSubmitting(false);
      hasSubmittedRef.current = false;
      onToast?.({ type: 'error', title: t('skills.meetingBots.couldNotStartTitle'), message });
    }
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={t('skills.meetingBots.modalAriaLabel')}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4"
      onClick={() => {
        if (canDismiss) onClose();
      }}>
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
            disabled={!canDismiss}
            className="absolute right-3 top-3 rounded-full p-1 text-stone-500 dark:text-neutral-400 hover:bg-white/80 dark:hover:bg-neutral-800/60 hover:text-stone-800 dark:hover:text-neutral-100 disabled:cursor-not-allowed disabled:opacity-40">
            ✕
          </button>
          <h2 className="text-base font-semibold text-stone-900 dark:text-neutral-100">
            {t('skills.meetingBots.modalTitle')}
          </h2>
          <p className="mt-1 text-xs leading-relaxed text-stone-600 dark:text-neutral-300">
            {t('skills.meetingBots.modalDesc')}
          </p>
        </div>

        <div className="space-y-4 p-5">
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
                placeholder={t('skills.meetingBots.platformHints.gmeet')}
                disabled={submitting}
                autoFocus
                className="mt-1 w-full rounded-xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-100 disabled:cursor-not-allowed disabled:bg-stone-50 dark:disabled:bg-neutral-800/60"
                required
              />
            </label>

            {/* PASSIVE MODE: the bot doesn't listen for a wake phrase or
                respond to a single participant — it just transcribes. The
                "Your Name in This Meeting" field is hidden so users aren't
                prompted for input that no longer affects behavior. Restore
                this block if the responsive bot is ever re-enabled. */}
            {/* <label className="block">
              <span className="text-[10px] font-medium uppercase tracking-wide text-stone-500 dark:text-neutral-400">
                {t('skills.meetingBots.respondToParticipant')}
              </span>
              <input
                type="text"
                autoComplete="off"
                spellCheck={false}
                value={respondTo}
                onChange={e => setRespondTo(e.target.value)}
                placeholder={t('skills.meetingBots.respondToParticipantHint')}
                disabled={submitting}
                required
                className="mt-1 w-full rounded-xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-100 disabled:cursor-not-allowed disabled:bg-stone-50 dark:disabled:bg-neutral-800/60"
              />
              <p className="mt-1 text-[10px] text-stone-400 dark:text-neutral-500">
                {t('skills.meetingBots.respondToParticipantDesc')}
              </p>
            </label> */}

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
                disabled={!canDismiss}
                className="rounded-xl px-3 py-2 text-sm font-medium text-stone-600 dark:text-neutral-300 hover:bg-stone-100 dark:hover:bg-neutral-800 disabled:cursor-not-allowed disabled:opacity-40">
                {t('common.cancel')}
              </button>
              <button
                type="submit"
                disabled={submitting || !meetUrl.trim()}
                className="rounded-xl bg-primary-500 px-4 py-2 text-sm font-semibold text-white hover:bg-primary-600 disabled:cursor-not-allowed disabled:bg-stone-200 dark:disabled:bg-neutral-700 disabled:text-stone-400 dark:disabled:text-neutral-500">
                {submitting
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
