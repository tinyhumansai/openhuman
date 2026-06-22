import { type RefObject, useCallback, useEffect, useMemo, useRef, useState } from 'react';

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
import { RecentCallsSection } from './RecentCallsSection';

type Toast = { type: 'success' | 'error' | 'info'; title: string; message?: string };

interface Props {
  onToast?: (toast: Toast) => void;
}

interface MeetingBotsInlineProps extends Props {
  hasSubmittedRef: RefObject<boolean>;
}

export default function MeetingBotsCard({ onToast }: Props) {
  const { t } = useT();
  const status = useAppSelector(selectBackendMeetStatus);
  const showActive = status === 'active';

  // `hasSubmittedRef` lives in this always-mounted parent so the success toast
  // fires reliably. When a join succeeds, `status` flips to 'active' and this
  // component swaps `MeetingBotsInline` → `ActiveMeetingView`, unmounting the
  // inline form before any effect inside it could observe 'active' (#3611
  // flattened these into a mutually-exclusive ternary). The inline form sets
  // this ref on submit; we fire the success toast here. The error path stays in
  // the inline form, which remains mounted during the 'error' state.
  const hasSubmittedRef = useRef(false);
  useEffect(() => {
    if (!hasSubmittedRef.current) return;
    if (status === 'active') {
      hasSubmittedRef.current = false;
      onToast?.({
        type: 'success',
        title: t('skills.meetingBots.joiningTitle'),
        message: t('skills.meetingBots.joiningMessage'),
      });
    }
  }, [status, onToast, t]);

  return showActive ? (
    <ActiveMeetingView onToast={onToast} />
  ) : (
    <MeetingBotsInline onToast={onToast} hasSubmittedRef={hasSubmittedRef} />
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

function MeetingBotsInline({ onToast, hasSubmittedRef }: MeetingBotsInlineProps) {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const [meetUrl, setMeetUrl] = useState('');
  // The participant the bot answers to (authorized speaker). Wired to the
  // backend join payload as `respondToParticipant` → `respondTo`, which the
  // meeting stream uses to gate in-call requests to this speaker only.
  const [respondTo, setRespondTo] = useState('');
  // Active (respond when addressed) vs listen-only (transcribe only). Defaults
  // to active; the bot still only replies after being addressed by the wake
  // phrase. Forwarded to the backend as `listenOnly` and mirrored into the
  // meet slice so the active view shows the right status.
  const [listenOnly, setListenOnly] = useState(false);
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
    void refreshRecentCalls();
    // This inline form remounts the instant a call ends, but the core writes
    // the call record asynchronously a few ms after the transcript arrives —
    // so the mount-time fetch can race ahead of that write and miss the just-
    // ended call. A couple of short delayed re-fetches reliably reflect it
    // without the user having to reopen the tab. Cheap (a ~2ms RPC each).
    const retries = [1200, 3000].map(delay =>
      setTimeout(() => void refreshRecentCalls(), delay)
    );
    return () => retries.forEach(clearTimeout);
  }, [refreshRecentCalls]);

  const selectedLabel = t('skills.meetingBots.platforms.gmeet');
  const agentName = personaDisplayName.trim() || 'Tiny';
  const systemPrompt = personaDescription.trim() || undefined;
  const mascotId = selectedMascotId ?? (mascotColor === 'custom' ? undefined : mascotColor);
  const riveColors =
    mascotColor === 'custom'
      ? { primaryColor: customPrimaryColor, secondaryColor: customSecondaryColor }
      : undefined;

  // Success ('active') is handled by the parent MeetingBotsCard, which stays
  // mounted across the inline→active view swap. The error path lives here
  // because the inline form remains mounted during the 'error' state and needs
  // to surface the failure inline (setError/setSubmitting) alongside the toast.
  useEffect(() => {
    if (!hasSubmittedRef.current) return;
    if (meetStatus === 'error') {
      hasSubmittedRef.current = false;
      const message = meetError?.trim() || t('skills.meetingBots.failedToStart');
      setError(message);
      setSubmitting(false);
      onToast?.({ type: 'error', title: t('skills.meetingBots.couldNotStartTitle'), message });
    }
  }, [meetStatus, meetError, onToast, t, hasSubmittedRef]);

  const handleSubmit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setError(null);
    setSubmitting(true);
    hasSubmittedRef.current = true;
    try {
      const meetingId = crypto.randomUUID();
      dispatch(setBackendMeetJoining({ meetUrl: meetUrl.trim(), meetingId, listenOnly }));
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
        listenOnly,
      });
    } catch (err) {
      const message = err instanceof Error ? err.message : t('skills.meetingBots.failedToStart');
      setError(message);
      setSubmitting(false);
      hasSubmittedRef.current = false;
      onToast?.({ type: 'error', title: t('skills.meetingBots.couldNotStartTitle'), message });
    }
  };

  return (
    <div className="rounded-2xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-4 shadow-soft animate-fade-up">
      <div className="mb-4">
        <h2 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
          {t('skills.meetingBots.modalTitle')}
        </h2>
        <p className="mt-1 text-xs leading-relaxed text-stone-600 dark:text-neutral-300">
          {t('skills.meetingBots.modalDesc')}
        </p>
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
            placeholder={t('skills.meetingBots.platformHints.gmeet')}
            disabled={submitting}
            className="mt-1 w-full rounded-xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-100 disabled:cursor-not-allowed disabled:bg-stone-50 dark:disabled:bg-neutral-800/60"
            required
          />
        </label>

        <label className="block">
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
        </label>

        <label className="flex items-start gap-3 rounded-xl border border-stone-200 dark:border-neutral-800 px-3 py-2.5">
          <input
            type="checkbox"
            checked={!listenOnly}
            onChange={e => setListenOnly(!e.target.checked)}
            disabled={submitting}
            className="mt-0.5 h-4 w-4 shrink-0 rounded border-stone-300 text-primary-500 focus:ring-2 focus:ring-primary-100 disabled:cursor-not-allowed"
          />
          <span className="min-w-0">
            <span className="block text-sm font-medium text-stone-800 dark:text-neutral-100">
              {t('skills.meetingBots.activeMode')}
            </span>
            <span className="mt-0.5 block text-[10px] leading-relaxed text-stone-400 dark:text-neutral-500">
              {t('skills.meetingBots.activeModeDesc')}
            </span>
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
            type="submit"
            disabled={submitting || !meetUrl.trim() || !respondTo.trim()}
            className="rounded-xl bg-primary-500 px-4 py-2 text-sm font-semibold text-white hover:bg-primary-600 disabled:cursor-not-allowed disabled:bg-stone-200 dark:disabled:bg-neutral-700 disabled:text-stone-400 dark:disabled:text-neutral-500">
            {submitting
              ? t('skills.meetingBots.starting')
              : t('skills.meetingBots.sendTo').replace('{label}', selectedLabel)}
          </button>
        </div>
      </form>

      <RecentCallsSection rows={recentCalls} error={recentError} />
    </div>
  );
}

