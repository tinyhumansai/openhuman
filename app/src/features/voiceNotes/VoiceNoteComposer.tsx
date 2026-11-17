/**
 * VoiceNoteComposer - record -> preview -> send, beside the text composer.
 *
 * Dictation (speech to text in the input box) is a separate control; this one
 * sends the recording itself as a voice note. The host owns what "send" means
 * (store the audio, transcribe it, append the row, start the turn) through
 * `onSend`; this component owns capture and the preview.
 */
import { MicIcon, SendIcon, SquareIcon, Trash2Icon } from 'lucide-react';
import { useCallback, useEffect, useMemo, useState } from 'react';

import Button from '../../components/ui/Button';
import { useT } from '../../lib/i18n/I18nContext';
import { useVoiceNoteRecorder, type VoiceNoteDraft } from './useVoiceNoteRecorder';
import { formatDuration } from './voiceNoteApi';
import { VoiceNotePlayer } from './VoiceNotePlayer';

export interface VoiceNoteComposerProps {
  disabled?: boolean;
  onSend: (draft: VoiceNoteDraft) => Promise<void>;
  onError?: (message: string) => void;
}

export function VoiceNoteComposer({ disabled, onSend, onError }: VoiceNoteComposerProps) {
  const { t } = useT();
  const recorder = useVoiceNoteRecorder();
  const { phase, draft, elapsedMs } = recorder;
  const [sending, setSending] = useState(false);

  const previewUrl = useMemo(() => (draft ? URL.createObjectURL(draft.blob) : null), [draft]);
  useEffect(
    () => () => {
      if (previewUrl) URL.revokeObjectURL(previewUrl);
    },
    [previewUrl]
  );

  const begin = useCallback(async () => {
    try {
      await recorder.start();
    } catch (err) {
      console.warn('[voice-note] microphone unavailable', err);
      onError?.(t('voiceNote.micDenied'));
    }
  }, [recorder, onError, t]);

  const send = useCallback(async () => {
    if (!draft || sending) return;
    setSending(true);
    try {
      await onSend(draft);
      recorder.discard();
    } catch (err) {
      console.warn('[voice-note] send failed', err);
      onError?.(err instanceof Error && err.message ? err.message : t('voiceNote.sendFailed'));
    } finally {
      setSending(false);
    }
  }, [draft, onSend, onError, recorder, sending, t]);

  if (phase === 'recording') {
    return (
      <div data-testid="voice-note-recording" className="flex items-center gap-1.5">
        <span className="bg-coral-500 size-2 animate-pulse rounded-full" aria-hidden="true" />
        <span role="status" className="text-xs tabular-nums">
          {t('voiceNote.recording')} {formatDuration(elapsedMs)}
        </span>
        <Button
          type="button"
          iconOnly
          size="xs"
          variant="secondary"
          analyticsId="voice-note-stop"
          data-testid="voice-note-stop"
          aria-label={t('voiceNote.stop')}
          onClick={recorder.stop}>
          <SquareIcon className="size-3.5" />
        </Button>
        <Button
          type="button"
          iconOnly
          size="xs"
          variant="tertiary"
          analyticsId="voice-note-discard"
          data-testid="voice-note-discard"
          aria-label={t('voiceNote.discard')}
          onClick={recorder.discard}>
          <Trash2Icon className="size-3.5" />
        </Button>
      </div>
    );
  }

  if (phase === 'recorded' && draft && previewUrl) {
    return (
      <div data-testid="voice-note-preview" className="flex items-center gap-1.5">
        <VoiceNotePlayer
          src={previewUrl}
          durationMs={draft.durationMs}
          waveform={draft.waveform}
          className="min-w-48"
        />
        <Button
          type="button"
          iconOnly
          size="xs"
          variant="primary"
          analyticsId="voice-note-send"
          data-testid="voice-note-send"
          aria-label={t('voiceNote.send')}
          disabled={disabled || sending}
          onClick={() => void send()}>
          <SendIcon className="size-3.5" />
        </Button>
        <Button
          type="button"
          iconOnly
          size="xs"
          variant="tertiary"
          analyticsId="voice-note-discard"
          data-testid="voice-note-discard"
          aria-label={t('voiceNote.discard')}
          disabled={sending}
          onClick={recorder.discard}>
          <Trash2Icon className="size-3.5" />
        </Button>
      </div>
    );
  }

  return (
    <Button
      type="button"
      iconOnly
      size="xs"
      variant="tertiary"
      analyticsId="voice-note-record"
      data-testid="voice-note-record"
      aria-label={t('voiceNote.record')}
      title={t('voiceNote.record')}
      disabled={disabled}
      onClick={() => void begin()}>
      <MicIcon className="size-4" />
    </Button>
  );
}

export default VoiceNoteComposer;
