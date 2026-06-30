/**
 * Live/active meeting view — shown when `backendMeet.status` is `'joining'`,
 * `'active'`, `'ended'`, or `'error'`.
 *
 * Extracted from `MeetingBotsCard` (previously `ActiveMeetingView`) to keep
 * each component within the repo's ~500-line guideline. Behavior is identical
 * to the original; it just lives in its own file now.
 */
import { useEffect, useMemo, useRef, useState } from 'react';

import { type MascotFace, RiveMascot } from '../../features/human/Mascot';
import { useT } from '../../lib/i18n/I18nContext';
import { leaveBackendMeetBot } from '../../services/meetCallService';
import {
  type BackendMeetHarnessEvent,
  type BackendMeetReplyEvent,
  type BackendMeetStatus,
  resetBackendMeet,
  selectBackendMeetLastHarness,
  selectBackendMeetLastReply,
  selectBackendMeetListenOnly,
  selectBackendMeetStatus,
  selectBackendMeetUrl,
} from '../../store/backendMeetSlice';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
import Button from '../ui/Button';
import { Spinner } from '../ui/icons';

type Toast = { type: 'success' | 'error' | 'info'; title: string; message?: string };

/**
 * Safety net for the pending "Leaving…" state. The leave RPC only confirms the
 * `bot:leave` event was enqueued on the socket — it does not wait for the bot to
 * actually leave. On the happy path `status` flips to ended/error (which unmounts
 * this banner) well within this window. If no such transition ever arrives (e.g.
 * the socket drops or the bot fails to emit its completion event), we re-enable
 * the button after this timeout so the user can retry instead of being stuck.
 */
export const LEAVE_SAFETY_TIMEOUT_MS = 10_000;

export interface ActiveMeetingBannerProps {
  onToast?: (toast: Toast) => void;
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

export function ActiveMeetingBanner({ onToast }: ActiveMeetingBannerProps) {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const status = useAppSelector(selectBackendMeetStatus);
  const meetUrl = useAppSelector(selectBackendMeetUrl);
  const listenOnly = useAppSelector(selectBackendMeetListenOnly);
  const lastReply = useAppSelector(selectBackendMeetLastReply);
  const lastHarness = useAppSelector(selectBackendMeetLastHarness);
  // selectBackendMeetError imported for parity; not used visually here — errors
  // surface in the composer's inline alert during the error state.
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
  const leaveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Clear the safety timer on unmount (the happy path: the banner unmounts when
  // `status` flips to ended/error). No setState here — just timer cleanup.
  useEffect(() => {
    return () => {
      if (leaveTimerRef.current) clearTimeout(leaveTimerRef.current);
    };
  }, []);

  const handleLeave = async () => {
    if (leaving) return;
    setLeaving(true);
    try {
      await leaveBackendMeetBot('user-requested');
      // Stay in the pending "Leaving…" state on success: when the bot actually
      // leaves, `status` flips to ended/error and this banner unmounts (the
      // parent only renders it while joining/active), so the flag never lingers.
      // The RPC only confirms the leave was enqueued, not that the bot left, so
      // arm a safety timeout that re-enables the button if no transition arrives.
      if (leaveTimerRef.current) clearTimeout(leaveTimerRef.current);
      leaveTimerRef.current = setTimeout(() => {
        leaveTimerRef.current = null;
        setLeaving(false);
      }, LEAVE_SAFETY_TIMEOUT_MS);
    } catch (err) {
      setLeaving(false);
      onToast?.({
        type: 'error',
        title: t('skills.meetingBots.couldNotLeaveTitle'),
        message: String(err),
      });
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
          <Button
            variant="secondary"
            size="sm"
            onClick={handleLeave}
            disabled={leaving}
            leadingIcon={leaving ? <Spinner className="h-3.5 w-3.5" /> : undefined}>
            {leaving ? t('skills.meetingBots.leavingButton') : t('skills.meetingBots.leaveButton')}
          </Button>
        )}
        {isDone && (
          <Button variant="secondary" size="sm" onClick={() => dispatch(resetBackendMeet())}>
            {t('common.close')}
          </Button>
        )}
      </div>
      <div className="flex items-center gap-4">
        <div className="w-20 h-20 flex-shrink-0">
          <RiveMascot face={face} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="text-sm font-semibold text-content">
            {t('skills.meetingBots.liveTitle')}
          </div>
          <div className="mt-0.5 text-xs text-content-muted">{statusText}</div>
          {meetingCode && (
            <div className="mt-1 truncate font-mono text-[11px] text-content-secondary">
              {meetingCode}
            </div>
          )}
          {lastReply?.reply && (
            <div className="mt-1.5 text-xs text-content-secondary line-clamp-2 italic">
              &ldquo;{lastReply.reply}&rdquo;
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
