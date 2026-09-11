import { ConversationProvider } from '@elevenlabs/react';
import { type RefObject, useEffect } from 'react';
import { LuMic, LuSquare } from 'react-icons/lu';

import Button from '../../components/ui/Button';
import { useT } from '../../lib/i18n/I18nContext';
import { useAppSelector } from '../../store/hooks';
import { selectEffectiveMascotVoiceId } from '../../store/mascotSlice';
import type { RealtimeVoiceAudio } from './voice/amplitudeLipsync';
import { useRealtimeVoiceSession } from './voice/useRealtimeVoiceSession';

/**
 * Realtime voice-chat controls for the Human tab (#5399). Rendered only when the
 * realtime voice mode is enabled and selected; the classic path is untouched.
 * Wraps its own `ConversationProvider` (required by `@elevenlabs/react`) so it
 * stays self-contained and adds no context to the rest of the app.
 */
function RealtimeVoiceControlsInner({
  audioRef,
  onSpeakingChange,
  appearance = 'button',
}: {
  audioRef?: RefObject<RealtimeVoiceAudio>;
  onSpeakingChange?: (speaking: boolean) => void;
  appearance?: RealtimeVoiceAppearance;
}) {
  const { t } = useT();
  const voiceId = useAppSelector(selectEffectiveMascotVoiceId);
  const session = useRealtimeVoiceSession({ voiceId });

  const active = session.state === 'active';
  const connecting = session.state === 'connecting';

  // Publish the output-loudness accessor for the mascot's lip-sync. Written into
  // a ref rather than lifted into state because the mascot samples it once per
  // animation frame; see `useAmplitudeLipsync`. The session lives under this
  // component's own ConversationProvider, so this is the only place that can
  // reach it.
  const { getOutputVolume, isSpeaking } = session;
  const speaking = active && isSpeaking;
  useEffect(() => {
    if (audioRef?.current) {
      audioRef.current.getOutputVolume = active ? getOutputVolume : null;
      audioRef.current.speaking = speaking;
    }
    // Surface the speaking edge so the page can gate the mascot's lip-sync rAF
    // loop (`useAmplitudeLipsync`'s `enabled`) — an idle or classic Human tab
    // then schedules no frames. Unlike the 60fps amplitude above, this flips a
    // couple of times per turn, so it is cheap to lift into React state.
    onSpeakingChange?.(speaking);
  }, [audioRef, active, speaking, getOutputVolume, onSpeakingChange]);

  // A session that ends mid-speech would otherwise leave `speaking` true and the
  // mouth frozen open — reset both the ref the mascot samples and the speaking
  // edge the page gates its loop on.
  useEffect(
    () => () => {
      if (audioRef?.current) {
        audioRef.current.getOutputVolume = null;
        audioRef.current.speaking = false;
      }
      onSpeakingChange?.(false);
    },
    [audioRef, onSpeakingChange]
  );

  const label = connecting
    ? t('voice.mode.connecting')
    : active
      ? t('voice.mode.stop')
      : t('voice.mode.start');

  const status = active
    ? session.isSpeaking
      ? t('voice.mode.speaking')
      : t('voice.mode.listening')
    : null;

  const toggle = () => (active ? session.stop() : void session.start());

  return (
    <div className="flex flex-col items-center gap-2" data-testid="realtime-voice-controls">
      {appearance === 'icon' ? (
        // The Human stage's single affordance: one circular mic that starts the
        // session and becomes a stop square while it runs. Same handler and
        // same `label` as the text button, so the accessible name still says
        // which of start/stop/connecting this press does.
        <Button
          iconOnly
          variant={active ? 'secondary' : 'primary'}
          analyticsId="human-realtime-voice-toggle"
          data-testid="human-voice-toggle"
          disabled={connecting}
          aria-pressed={active}
          aria-label={label}
          title={label}
          className="size-14 rounded-full shadow-float"
          onClick={toggle}>
          {active ? <LuSquare className="size-5 fill-current" /> : <LuMic className="size-6" />}
        </Button>
      ) : (
        <Button
          analyticsId="human-realtime-voice-toggle"
          disabled={connecting}
          aria-label={label}
          onClick={toggle}>
          {label}
        </Button>
      )}
      {status && <span className="text-xs text-content-muted">{status}</span>}
      {session.error && (
        <span className="text-xs text-red-600 dark:text-red-300" role="alert">
          {session.error}
        </span>
      )}
    </div>
  );
}

/**
 * How the toggle presents itself.
 *
 * - `button` — the labelled text button, for surfaces that sit in a row of
 *   other controls.
 * - `icon` — a circular mic (stop square while running), for the Human stage
 *   where it is the only control on the page.
 */
export type RealtimeVoiceAppearance = 'button' | 'icon';

export default function RealtimeVoiceControls({
  audioRef,
  onSpeakingChange,
  appearance,
}: {
  /** Optional sink for the mascot's lip-sync signal (see RealtimeVoiceAudio). */
  audioRef?: RefObject<RealtimeVoiceAudio>;
  /** Notified with the agent's speaking edge so the page can gate the loop. */
  onSpeakingChange?: (speaking: boolean) => void;
  /** Presentation of the start/stop toggle. Defaults to the labelled button. */
  appearance?: RealtimeVoiceAppearance;
}) {
  return (
    <ConversationProvider>
      <RealtimeVoiceControlsInner
        audioRef={audioRef}
        onSpeakingChange={onSpeakingChange}
        appearance={appearance}
      />
    </ConversationProvider>
  );
}
