import debug from 'debug';
import { type RefObject, useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { type MascotFace, RiveMascot } from '../../features/human/Mascot';
import type { MascotColor } from '../../features/human/Mascot/mascotPalette';
import { useComposioIntegrations } from '../../lib/composio/hooks';
import type { ComposioConnection } from '../../lib/composio/types';
import { useT } from '../../lib/i18n/I18nContext';
import {
  isCapacityGateMessage,
  joinMeetViaBackendBot,
  leaveBackendMeetBot,
  listMeetCalls,
  type MeetCallRecord,
  type MeetingPlatform,
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
import Button from '../ui/Button';
import { RecentCallsSection } from './RecentCallsSection';

type Toast = { type: 'success' | 'error' | 'info'; title: string; message?: string };

const log = debug('meeting-bots');
const MEETING_BOT_MASCOT_IDS = new Set(['yellow', 'blue', 'burgundy', 'black', 'navy']);

function resolveMeetingBotMascotId(
  selectedMascotId: string | null,
  mascotColor: MascotColor
): string | undefined {
  if (selectedMascotId && MEETING_BOT_MASCOT_IDS.has(selectedMascotId)) return selectedMascotId;
  if (mascotColor !== 'custom') return mascotColor;
  return undefined;
}

// Composio only hands back a connected account's email — there is no separate
// display-name field on `ComposioConnection`. A meeting display name is almost
// always "First Last" derived from that account, so we best-effort humanize the
// email's local part (`first.last` → `First Last`).
function deriveDisplayNameFromEmail(email: string | undefined): string {
  const localPart = email?.split('@')[0]?.trim();
  if (!localPart) return '';
  return localPart
    .split(/[._-]+/)
    .filter(Boolean)
    .map(token => token.charAt(0).toUpperCase() + token.slice(1))
    .join(' ');
}

// Per-platform priority of Composio toolkits to source the user's in-call
// display name from: the platform's own connected account first, then the
// mailbox, then blank. Slugs are canonical Composio slugs (see
// `canonicalizeComposioToolkitSlug`).
const NAME_SOURCE_TOOLKITS: Record<MeetingPlatform, string[]> = {
  gmeet: ['googlemeet', 'gmail'],
  zoom: ['zoom', 'gmail'],
  teams: ['microsoft_teams', 'outlook', 'gmail'],
  webex: ['webex', 'gmail'],
};

// Resolve a default "Your name in this meeting" for the given platform: walk
// that platform's toolkit priority (own account → mail → blank) and return the
// first connected account whose email yields a usable name; blank if none are
// connected. The single entry point the form calls.
function resolveMeetingDisplayName(
  platform: MeetingPlatform,
  connectionByToolkit: Map<string, ComposioConnection>
): string {
  for (const slug of NAME_SOURCE_TOOLKITS[platform]) {
    const name = deriveDisplayNameFromEmail(connectionByToolkit.get(slug)?.accountEmail);
    if (name) return name;
  }
  return '';
}

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
          <Button variant="secondary" size="sm" onClick={handleLeave} disabled={leaving}>
            {t('skills.meetingBots.leaveButton')}
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

function MeetingBotsInline({ onToast, hasSubmittedRef }: MeetingBotsInlineProps) {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const [meetUrl, setMeetUrl] = useState('');
  // The participant the bot answers to (authorized speaker). Wired to the
  // backend join payload as `respondToParticipant` → `respondTo`, which the
  // meeting stream uses to gate in-call requests to this speaker only.
  const [respondTo, setRespondTo] = useState('');
  // Once the user types in the name field we stop auto-prefilling it, so a
  // late-arriving Composio fetch (it polls) can never clobber manual input.
  const respondToTouchedRef = useRef(false);
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
  // The meeting platform this form sends to (only Google Meet is wired up for
  // now). Drives both the join payload and which connected accounts we source
  // the default display name from.
  const platform: MeetingPlatform = 'gmeet';
  // Prefill "Your name in this meeting" from the connected account that best
  // matches this platform (calendar → mail → platform-native).
  const { connectionByToolkit } = useComposioIntegrations();
  const resolvedDisplayName = resolveMeetingDisplayName(platform, connectionByToolkit);

  useEffect(() => {
    if (respondToTouchedRef.current || !resolvedDisplayName) return;
    setRespondTo(prev => (prev.trim() ? prev : resolvedDisplayName));
  }, [resolvedDisplayName]);

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
    const retries = [1200, 3000].map(delay => setTimeout(() => void refreshRecentCalls(), delay));
    return () => retries.forEach(clearTimeout);
  }, [refreshRecentCalls]);

  const selectedLabel = t('skills.meetingBots.platforms.gmeet');
  const agentName = personaDisplayName.trim() || 'Tiny';
  const systemPrompt = personaDescription.trim() || undefined;
  const mascotId = resolveMeetingBotMascotId(selectedMascotId, mascotColor);
  const riveColors =
    mascotColor === 'custom'
      ? { primaryColor: customPrimaryColor, secondaryColor: customSecondaryColor }
      : undefined;
  const wakePhrase = listenOnly ? undefined : `Hey ${agentName}`;

  // Success ('active') is handled by the parent MeetingBotsCard, which stays
  // mounted across the inline→active view swap. The error path lives here
  // because the inline form remains mounted during the 'error' state and needs
  // to surface the failure inline (setError/setSubmitting) alongside the toast.
  useEffect(() => {
    if (!hasSubmittedRef.current) return;
    if (meetStatus === 'error') {
      hasSubmittedRef.current = false;
      const raw = meetError?.trim() || t('skills.meetingBots.failedToStart');
      // A capacity-gate error carries the backend's terse "…try again later."
      // wording; show the tailored, actionable (and localized) copy instead (#4151).
      const message = isCapacityGateMessage(raw)
        ? t('skills.meetingBots.serverOverloaded')
        : raw;
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
      log('join submit %o', {
        active: !listenOnly,
        agentChars: agentName.length,
        ownerChars: respondTo.trim().length,
        wakeChars: wakePhrase?.length ?? 0,
        correlationId: meetingId,
      });
      dispatch(setBackendMeetJoining({ meetUrl: meetUrl.trim(), meetingId, listenOnly }));
      await joinMeetViaBackendBot({
        meetUrl,
        displayName: agentName,
        platform,
        agentName,
        systemPrompt,
        mascotId,
        riveColors,
        correlationId: meetingId,
        respondToParticipant: respondTo.trim() || undefined,
        wakePhrase,
        listenOnly,
      });
    } catch (err) {
      const raw = err instanceof Error ? err.message : t('skills.meetingBots.failedToStart');
      const message = isCapacityGateMessage(raw)
        ? t('skills.meetingBots.serverOverloaded')
        : raw;
      setError(message);
      setSubmitting(false);
      hasSubmittedRef.current = false;
      onToast?.({ type: 'error', title: t('skills.meetingBots.couldNotStartTitle'), message });
    }
  };

  return (
    <div className="rounded-2xl border border-line bg-surface p-4 shadow-soft animate-fade-up">
      <div className="mb-4">
        <h2 className="text-sm font-semibold text-content">
          {t('skills.meetingBots.modalTitle')}
        </h2>
        <p className="mt-1 text-xs leading-relaxed text-content-secondary">
          {t('skills.meetingBots.modalDesc')}
        </p>
      </div>

      <form onSubmit={handleSubmit} className="space-y-3">
        <label className="block">
          <span className="text-[10px] font-medium uppercase tracking-wide text-content-muted">
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
            className="mt-1 w-full rounded-xl border border-line bg-surface px-3 py-2 text-sm text-content placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-100 disabled:cursor-not-allowed disabled:bg-surface-muted dark:disabled:bg-surface-muted/60"
            required
          />
        </label>

        <label className="block">
          <span className="text-[10px] font-medium uppercase tracking-wide text-content-muted">
            {t('skills.meetingBots.respondToParticipant')}
          </span>
          <input
            type="text"
            autoComplete="off"
            spellCheck={false}
            value={respondTo}
            onChange={e => {
              respondToTouchedRef.current = true;
              setRespondTo(e.target.value);
            }}
            placeholder={t('skills.meetingBots.respondToParticipantHint')}
            disabled={submitting}
            required
            className="mt-1 w-full rounded-xl border border-line bg-surface px-3 py-2 text-sm text-content placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-100 disabled:cursor-not-allowed disabled:bg-surface-muted dark:disabled:bg-surface-muted/60"
          />
          <p className="mt-1 text-[10px] text-content-faint">
            {t('skills.meetingBots.respondToParticipantDesc')}
          </p>
        </label>

        <label className="flex items-start gap-3 rounded-xl border border-line px-3 py-2.5">
          <input
            type="checkbox"
            checked={!listenOnly}
            onChange={e => setListenOnly(!e.target.checked)}
            disabled={submitting}
            className="mt-0.5 h-4 w-4 shrink-0 rounded border-line-strong text-primary-500 focus:ring-2 focus:ring-primary-100 disabled:cursor-not-allowed"
          />
          <span className="min-w-0">
            <span className="block text-sm font-medium text-content">
              {t('skills.meetingBots.activeMode')}
            </span>
            <span className="mt-0.5 block text-[10px] leading-relaxed text-content-faint">
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
          <Button
            type="submit"
            variant="primary"
            disabled={submitting || !meetUrl.trim() || !respondTo.trim()}>
            {submitting
              ? t('skills.meetingBots.starting')
              : t('skills.meetingBots.sendTo').replace('{label}', selectedLabel)}
          </Button>
        </div>
      </form>

      <RecentCallsSection rows={recentCalls} error={recentError} />
    </div>
  );
}
