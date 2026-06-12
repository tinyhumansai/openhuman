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
  }, [refreshRecentCalls]);

  const selectedLabel = t('skills.meetingBots.platforms.gmeet');
  const agentName = personaDisplayName.trim() || 'OpenHuman';
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
      dispatch(setBackendMeetJoining({ meetUrl: meetUrl.trim(), meetingId }));
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
  );
}

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
