import { useT } from '@/lib/i18n/I18nContext';
import { useComposerDictationState } from '@/providers/ComposerDictationContext';
import { ComposerPrimitive, useAuiState } from '@assistant-ui/react';
import { LoaderCircleIcon, MicIcon, SquareIcon, XIcon } from 'lucide-react';

import { TooltipIconButton } from './tooltip-icon-button';

/** Offer capture only when supported, with explicit Finish and Discard actions. */
export function ComposerDictationControls() {
  const { t } = useT();
  const dictation = useComposerDictationState();
  const isRunning = useAuiState(state => state.thread.isRunning);
  if (!dictation?.adapter) return null;

  const active = ['starting', 'recording', 'transcribing'].includes(dictation.status);
  const label = t('composer.dictate', 'Dictate');
  const finishLabel = t('composer.finishDictation', 'Finish dictation');
  const discardLabel = t('composer.discardDictation', 'Discard dictation');

  return (
    <>
      {active ? (
        <>
          {dictation.status === 'recording' && (
            <ComposerPrimitive.StopDictation asChild>
              <TooltipIconButton
                tooltip={finishLabel}
                aria-label={finishLabel}
                type="button"
                className="aui-composer-stop-dictation text-destructive size-7 rounded-full">
                <SquareIcon className="size-3.5 fill-current" />
              </TooltipIconButton>
            </ComposerPrimitive.StopDictation>
          )}
          {dictation.status !== 'recording' && (
            <LoaderCircleIcon className="text-muted-foreground size-4 animate-spin" aria-hidden />
          )}
          <TooltipIconButton
            tooltip={discardLabel}
            aria-label={discardLabel}
            type="button"
            className="aui-composer-discard-dictation size-7 rounded-full"
            onClick={dictation.cancel}>
            <XIcon className="size-4" />
          </TooltipIconButton>
        </>
      ) : (
        <ComposerPrimitive.Dictate asChild>
          <TooltipIconButton
            tooltip={label}
            aria-label={label}
            type="button"
            disabled={isRunning}
            className="aui-composer-dictate text-muted-foreground hover:text-foreground size-7 rounded-full">
            <MicIcon className="size-4" />
          </TooltipIconButton>
        </ComposerPrimitive.Dictate>
      )}
    </>
  );
}

/** Announce dictation phases and translate every safe error code for the user. */
export function ComposerDictationStatus() {
  const { t } = useT();
  const dictation = useComposerDictationState();
  if (!dictation) return null;

  let errorText: string | null = null;
  switch (dictation.error) {
    case 'stt-unavailable':
      errorText = t(
        'composer.dictationUnavailable',
        'Dictation is unavailable. Check your speech provider in Settings > Voice.'
      );
      break;
    case 'voice-status-failed':
      errorText = t(
        'composer.dictationStatusFailed',
        'Could not check dictation availability. Return to this window to try again.'
      );
      break;
    case 'microphone-unavailable':
      errorText = t('mic.unavailable', 'Microphone is not available');
      break;
    case 'permission-denied':
      errorText = t('mic.permissionDenied', 'Microphone permission denied');
      break;
    case 'device-unavailable':
      errorText = t(
        'mic.deviceUnavailable',
        'Selected microphone is unavailable. Try a different device.'
      );
      break;
    case 'device-in-use':
      errorText = t('mic.deviceInUse', 'Microphone is in use by another application.');
      break;
    case 'recorder-failed':
      errorText = t('mic.failedToStartRecorder', 'Failed to start recorder');
      break;
    case 'no-audio':
      errorText = t('mic.noAudioCaptured', 'No audio captured');
      break;
    case 'no-speech':
      errorText = t('mic.noSpeechDetected', 'No speech detected');
      break;
    case 'transcription-failed':
      errorText = t('composer.dictationFailed', 'Transcription failed. Please try again.');
      break;
    case 'timed-out':
      errorText = t('composer.dictationTimedOut', 'Dictation timed out. Please try again.');
      break;
    case 'voice-unavailable':
      errorText = t(
        'mic.voiceNotCompiled',
        'Voice transcription is not included in this version of the app. Update OpenHuman to enable it.'
      );
      break;
  }
  if (errorText) {
    return (
      <p role="alert" className="text-destructive px-2.5 py-1 text-sm">
        {errorText}
      </p>
    );
  }

  const statusText =
    dictation.status === 'starting'
      ? t('composer.dictationStarting', 'Starting microphone...')
      : dictation.status === 'recording'
        ? t('composer.dictationRecording', 'Recording. Finish to add text, or discard.')
        : dictation.status === 'transcribing'
          ? t('mic.transcribing', 'Transcribing...')
          : null;

  return statusText ? (
    <p role="status" className="text-muted-foreground px-2.5 py-1 text-sm">
      {statusText}
    </p>
  ) : null;
}
