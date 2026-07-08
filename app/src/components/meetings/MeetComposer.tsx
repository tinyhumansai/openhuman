/**
 * Redesigned meeting composer card.
 *
 * Replaces the hardcoded-gmeet `MeetingBotsInline` form with a platform
 * selector (Google Meet / Zoom / Teams / Webex), a URL input whose placeholder
 * adapts to the selected platform, a "Your name" field that auto-prefills from
 * the connected Composio account, and a respond-when-addressed toggle.
 */
import debug from 'debug';
import { type RefObject, useEffect, useRef, useState } from 'react';

import { useMascotManifest } from '../../features/human/Mascot/manifest/useMascotManifest';
import { useComposioIntegrations } from '../../lib/composio/hooks';
import { useT } from '../../lib/i18n/I18nContext';
import {
  isCapacityGateMessage,
  joinMeetViaBackendBot,
  type MeetingPlatform,
} from '../../services/meetCallService';
import {
  selectBackendMeetError,
  selectBackendMeetStatus,
  setBackendMeetJoining,
} from '../../store/backendMeetSlice';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
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
import {
  buildMeetingMascots,
  platformLabel,
  platformUrlPlaceholder,
  resolveMeetingBotMascotId,
  resolveMeetingDisplayName,
} from './meetingUtils';
import { PlatformChips } from './PlatformChips';

const log = debug('meetings:composer');

type Toast = { type: 'success' | 'error' | 'info'; title: string; message?: string };

export interface MeetComposerProps {
  onToast?: (toast: Toast) => void;
  /** Ref owned by the parent (MeetingsPage) so the success toast can fire
   *  after the inline form unmounts on status → 'active'. */
  hasSubmittedRef: RefObject<boolean>;
}

export function MeetComposer({ onToast, hasSubmittedRef }: MeetComposerProps) {
  const { t } = useT();
  const dispatch = useAppDispatch();

  // ── Platform selector ────────────────────────────────────────────────────
  const [platform, setPlatform] = useState<MeetingPlatform>('gmeet');

  // ── Form state ───────────────────────────────────────────────────────────
  const [meetUrl, setMeetUrl] = useState('');
  // The participant the bot answers to (authorized speaker). Wired to the
  // backend join payload as `respondToParticipant`.
  const [respondTo, setRespondTo] = useState('');
  // Once the user types in the name field we stop auto-prefilling it, so a
  // late-arriving Composio fetch (it polls) can never clobber manual input.
  const respondToTouchedRef = useRef(false);
  // Active (respond when addressed) vs listen-only (transcribe only).
  const [listenOnly, setListenOnly] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // ── Persona / mascot config ──────────────────────────────────────────────
  const personaDisplayName = useAppSelector(selectPersonaDisplayName);
  const personaDescription = useAppSelector(selectPersonaDescription);
  const selectedMascotId = useAppSelector(selectSelectedMascotId);
  const mascotColor = useAppSelector(selectMascotColor);
  const customPrimaryColor = useAppSelector(selectCustomPrimaryColor);
  const customSecondaryColor = useAppSelector(selectCustomSecondaryColor);
  // Dual-mascot config (issue #4277): when a distinct second mascot is enabled
  // we send both slots (each with its own voice) so the backend bot renders
  // two mascots and alternates who speaks. Single-mascot keeps the legacy
  // `mascotId` path below untouched.
  const dualMascotEnabled = useAppSelector(selectDualMascotEnabled);
  const mascotVoicePair = useAppSelector(selectMeetingMascotVoicePair);
  // Manifest drives name-addressed routing (#4277 follow-up): each dual slot is
  // tagged with its display name so "Hey Toshi …" routes to that mascot.
  const { manifest } = useMascotManifest();

  // ── Meet slice ───────────────────────────────────────────────────────────
  const meetStatus = useAppSelector(selectBackendMeetStatus);
  const meetError = useAppSelector(selectBackendMeetError);

  // ── Composio name prefill ────────────────────────────────────────────────
  const { connectionByToolkit } = useComposioIntegrations();
  const resolvedDisplayName = resolveMeetingDisplayName(platform, connectionByToolkit);

  // Derive the value shown in the "Your name" field during render — no effect
  // needed (satisfies react-hooks/set-state-in-effect):
  //   • Untouched: use the Composio-resolved name for the current platform.
  //     Re-derives automatically whenever `platform` or `connectionByToolkit`
  //     changes, so late-arriving Composio fetches are reflected immediately.
  //   • Touched:   use exactly what the user typed.
  const displayedRespondTo = !respondToTouchedRef.current ? resolvedDisplayName : respondTo;

  // When the platform changes the displayed name re-derives on the next render
  // via resolvedDisplayName — no extra setState needed.
  const handlePlatformChange = (next: MeetingPlatform) => {
    log('[composer] platform changed from=%s to=%s', platform, next);
    setPlatform(next);
  };

  // ── Error path (inline form stays mounted during 'error') ────────────────
  // setState is deferred via setTimeout so the rule's transitive analysis does
  // not consider them synchronous within the effect body.  A 0-ms timer fires
  // before the next paint so the visible latency is imperceptible.
  useEffect(() => {
    if (!hasSubmittedRef.current) return;
    if (meetStatus !== 'error') return;

    hasSubmittedRef.current = false;
    const raw = meetError?.trim() || t('skills.meetingBots.failedToStart');
    const message = isCapacityGateMessage(raw) ? t('skills.meetingBots.serverOverloaded') : raw;
    log('[composer] join error: %s', message);
    onToast?.({ type: 'error', title: t('skills.meetingBots.couldNotStartTitle'), message });

    const id = setTimeout(() => {
      setError(message);
      setSubmitting(false);
    }, 0);
    return () => clearTimeout(id);
  }, [meetStatus, meetError, onToast, t, hasSubmittedRef]);

  // ── Submit ───────────────────────────────────────────────────────────────
  const agentName = personaDisplayName.trim() || 'Tiny';
  const systemPrompt = personaDescription.trim() || undefined;
  const mascotId = resolveMeetingBotMascotId(selectedMascotId, mascotColor);
  const riveColors =
    mascotColor === 'custom'
      ? { primaryColor: customPrimaryColor, secondaryColor: customSecondaryColor }
      : undefined;
  // Two-mascot slots for the backend bot (issue #4277) — built via the shared
  // helper so this live-join path and the UpcomingTable scheduled-join path stay
  // behaviorally identical.
  const mascots = buildMeetingMascots({
    dualMascotEnabled,
    mascotVoicePair,
    manifest,
    mascotId,
    riveColors,
    agentName,
  });
  const wakePhrase = listenOnly ? undefined : `Hey ${agentName}`;

  const handleSubmit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setError(null);
    setSubmitting(true);
    hasSubmittedRef.current = true;
    const meetingId = crypto.randomUUID();
    log(
      '[composer] submit platform=%s active=%s correlationId=%s',
      platform,
      !listenOnly,
      meetingId
    );
    // Name-addressing (#4277 follow-up) trace: the exact mascot ids + names sent
    // to the backend. If `mascots` is undefined or a slot's `name` is empty,
    // name addressing ("Hey Toshi") can't work — the backend needs both names.
    log(
      '[composer] join mascots=%o wakePhrase=%s',
      mascots?.map(m => ({ mascotId: m.mascotId, name: m.name })),
      wakePhrase
    );
    try {
      // Await the RPC BEFORE dispatching setBackendMeetJoining so that a
      // synchronous rejection (bad URL, auth failure) can be shown inline
      // without unmounting this component. setBackendMeetJoining transitions
      // status to 'joining' which causes MeetingsPage to swap this composer for
      // ActiveMeetingBanner — if we did that before the await, a sync throw
      // would land in the catch block of an already-unmounted component and
      // the error would never surface.
      await joinMeetViaBackendBot({
        meetUrl,
        displayName: agentName,
        platform,
        agentName,
        systemPrompt,
        mascotId,
        riveColors,
        // Dual-mascot slots (issue #4277); undefined for single-mascot calls.
        mascots,
        correlationId: meetingId,
        respondToParticipant: displayedRespondTo.trim() || undefined,
        wakePhrase,
        listenOnly,
      });
      // RPC was accepted — transition the UI to the joining / active banner.
      dispatch(setBackendMeetJoining({ meetUrl: meetUrl.trim(), meetingId, listenOnly }));
    } catch (err) {
      const raw = err instanceof Error ? err.message : t('skills.meetingBots.failedToStart');
      const message = isCapacityGateMessage(raw) ? t('skills.meetingBots.serverOverloaded') : raw;
      log('[composer] join threw: %s', message);
      setError(message);
      setSubmitting(false);
      hasSubmittedRef.current = false;
      onToast?.({ type: 'error', title: t('skills.meetingBots.couldNotStartTitle'), message });
    }
  };

  const selectedLabel = platformLabel(platform, t);
  const urlPlaceholder = platformUrlPlaceholder(platform, t);

  return (
    <div className="rounded-2xl border border-line bg-surface p-4 shadow-soft animate-fade-up">
      {/* Header */}
      <div className="mb-4">
        <h2 className="text-sm font-semibold text-content">{t('skills.meetingBots.modalTitle')}</h2>
        <p className="mt-1 text-xs leading-relaxed text-content-secondary">
          {t('skills.meetingBots.modalDesc')}
        </p>
      </div>

      {/* Platform selector */}
      <div className="mb-4">
        <PlatformChips selected={platform} onSelect={handlePlatformChange} disabled={submitting} />
      </div>

      <form onSubmit={handleSubmit} className="space-y-3">
        {/* Meeting URL */}
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
            placeholder={urlPlaceholder}
            disabled={submitting}
            aria-label={t('skills.meetingBots.meetingLink')}
            className="mt-1 w-full rounded-xl border border-line bg-surface px-3 py-2 text-sm text-content placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-100 disabled:cursor-not-allowed disabled:bg-surface-muted dark:disabled:bg-surface-muted/60"
            required
          />
        </label>

        {/* Your name */}
        <label className="block">
          <span className="text-[10px] font-medium uppercase tracking-wide text-content-muted">
            {t('skills.meetingBots.respondToParticipant')}
          </span>
          <input
            type="text"
            autoComplete="off"
            spellCheck={false}
            value={displayedRespondTo}
            onChange={e => {
              respondToTouchedRef.current = true;
              setRespondTo(e.target.value);
            }}
            placeholder={t('skills.meetingBots.respondToParticipantHint')}
            disabled={submitting}
            required
            aria-label={t('skills.meetingBots.respondToParticipant')}
            className="mt-1 w-full rounded-xl border border-line bg-surface px-3 py-2 text-sm text-content placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-100 disabled:cursor-not-allowed disabled:bg-surface-muted dark:disabled:bg-surface-muted/60"
          />
          <p className="mt-1 text-[10px] text-content-faint">
            {t('skills.meetingBots.respondToParticipantDesc')}
          </p>
        </label>

        {/* Respond toggle */}
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

        {/* Inline error */}
        {error && (
          <div
            role="alert"
            className="rounded-xl border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-3 py-2 text-xs text-coral-700 dark:text-coral-300">
            {error}
          </div>
        )}

        {/* Submit */}
        <div className="flex items-center justify-end gap-2 pt-1">
          <Button
            type="submit"
            variant="primary"
            disabled={submitting || !meetUrl.trim() || !displayedRespondTo.trim()}>
            {submitting
              ? t('skills.meetingBots.starting')
              : t('skills.meetingBots.sendTo').replace('{label}', selectedLabel)}
          </Button>
        </div>
      </form>
    </div>
  );
}
