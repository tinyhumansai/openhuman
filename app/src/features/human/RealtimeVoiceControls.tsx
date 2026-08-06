import { ConversationProvider } from '@elevenlabs/react';

import Button from '../../components/ui/Button';
import { useT } from '../../lib/i18n/I18nContext';
import { useAppSelector } from '../../store/hooks';
import { selectEffectiveMascotVoiceId } from '../../store/mascotSlice';
import { useRealtimeVoiceSession } from './voice/useRealtimeVoiceSession';

/**
 * Realtime voice-chat controls for the Human tab (#5399). Rendered only when the
 * realtime voice mode is enabled and selected; the classic path is untouched.
 * Wraps its own `ConversationProvider` (required by `@elevenlabs/react`) so it
 * stays self-contained and adds no context to the rest of the app.
 */
function RealtimeVoiceControlsInner() {
  const { t } = useT();
  const voiceId = useAppSelector(selectEffectiveMascotVoiceId);
  const session = useRealtimeVoiceSession({ voiceId });

  const active = session.state === 'active';
  const connecting = session.state === 'connecting';

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

  return (
    <div className="flex flex-col items-center gap-2" data-testid="realtime-voice-controls">
      <Button
        analyticsId="human-realtime-voice-toggle"
        disabled={connecting}
        aria-label={label}
        onClick={() => (active ? session.stop() : void session.start())}>
        {label}
      </Button>
      {status && <span className="text-xs text-content-muted">{status}</span>}
      {session.error && (
        <span className="text-xs text-red-600 dark:text-red-300" role="alert">
          {session.error}
        </span>
      )}
    </div>
  );
}

export default function RealtimeVoiceControls() {
  return (
    <ConversationProvider>
      <RealtimeVoiceControlsInner />
    </ConversationProvider>
  );
}
