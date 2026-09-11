import { useCallback, useSyncExternalStore } from 'react';

import { SettingsSwitch } from '../../../components/settings/controls';
import Button from '../../../components/ui/Button';
import { useT } from '../../../lib/i18n/I18nContext';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import {
  selectSpeakReplies,
  selectVoiceMode,
  setChatMascotListening,
  setSpeakReplies,
} from '../../../store/mascotSlice';
import { VOICE_MODE_FLAG_ENABLED } from '../../../utils/config';
import MicComposer from '../MicComposer';
import RealtimeVoiceControls from '../RealtimeVoiceControls';
import { useChatMascot } from './ChatMascotContext';

/**
 * The scaled-up mascot surface: the former Human page, folded into the chat's
 * right-hand column.
 *
 * The mascot itself is **not** rendered here — `ChatMascotOverlay` paints the
 * one shared instance over the `stageRef` placeholder. What lives here is the
 * voice interaction that only makes sense while expanded: the mic, the input
 * device selector, the speak-replies switch, and the collapse control.
 *
 * The chat's text composer stays live in the left column throughout, so the
 * user can type or talk without leaving this state.
 */
const ChatMascotStage = () => {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const { stageRef, collapse, sendStore } = useChatMascot();
  const speakReplies = useAppSelector(selectSpeakReplies);
  // Realtime voice agents (#5399) landed on the Human page while this surface
  // was replacing it, so the gate moves here rather than being dropped: the
  // stage IS the voice surface now. Same two conditions as before — build flag
  // on AND the persisted mode set to realtime — so it still ships dark.
  const voiceMode = useAppSelector(selectVoiceMode);
  const realtimeEnabled = VOICE_MODE_FLAG_ENABLED && voiceMode === 'realtime';

  // Subscribing to the store (rather than reading a context field) keeps the
  // chat tree out of this component's update path — see ChatMascotContext.
  const binding = useSyncExternalStore(sendStore.subscribe, sendStore.get, sendStore.get);

  const handleSubmit = useCallback(
    async (text: string) => {
      await binding?.submit(text);
    },
    [binding]
  );

  const handleError = useCallback(
    (message: string) => {
      binding?.onError(message);
    },
    [binding]
  );

  const handleRecordingChange = useCallback(
    (recording: boolean) => {
      dispatch(setChatMascotListening(recording));
    },
    [dispatch]
  );

  return (
    <div
      className="flex h-full min-h-0 flex-col items-center justify-center gap-4 overflow-hidden rounded-2xl border border-line/70 bg-surface-muted px-3 py-4 dark:bg-surface/60"
      data-testid="chat-mascot-stage">
      <div className="flex w-full items-center justify-end">
        <Button
          iconOnly
          variant="tertiary"
          onClick={collapse}
          aria-label={t('chat.mascot.collapse')}
          title={t('chat.mascot.collapse')}
          analyticsId="chat-mascot-collapse"
          data-testid="chat-mascot-collapse"
          className="rounded-full">
          <svg className="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={1.8}
              d="M9 9L4 4m0 0v5m0-5h5m6 6l5 5m0 0v-5m0 5h-5"
            />
          </svg>
        </Button>
      </div>

      {/* Mascot stage — an empty anchor. ChatMascotOverlay paints over it so the
          same Rive instance survives the dock ⇄ stage transition.

          Clickable so the mascot toggles both ways: the dock expands it, the
          mascot itself sends it back. `aria-hidden` + `tabIndex={-1}` on purpose
          — this is a redundant pointer target layered under the art, and the
          collapse button above is the one labelled control. Exposing both would
          announce the same action twice. */}
      <button
        ref={node => {
          stageRef.current = node;
        }}
        type="button"
        aria-hidden="true"
        tabIndex={-1}
        onClick={collapse}
        className="w-full max-w-[420px] flex-1 min-h-0 cursor-pointer"
        data-testid="chat-mascot-stage-anchor"
        data-analytics-id="chat-mascot-toggle"
      />

      {realtimeEnabled ? (
        <RealtimeVoiceControls />
      ) : (
        <MicComposer
          // Mirrors the mic-cloud call site in Conversations: without the
          // binding's own `disabled` (which folds in `!selectedThreadId`) a mic
          // submit before a thread exists is silently dropped.
          disabled={binding == null || binding.disabled}
          onSubmit={handleSubmit}
          onError={handleError}
          onRecordingChange={handleRecordingChange}
          showDeviceSelector
        />
      )}

      <label
        htmlFor="chat-mascot-speak-replies"
        className="flex cursor-pointer select-none items-center gap-2.5 text-xs text-content-secondary">
        <SettingsSwitch
          id="chat-mascot-speak-replies"
          checked={speakReplies}
          onCheckedChange={next => dispatch(setSpeakReplies(next))}
          aria-label={t('chat.mascot.speakReplies')}
          data-testid="chat-mascot-speak-replies"
        />
        <span>{t('chat.mascot.speakReplies')}</span>
      </label>
      <p className="max-w-[280px] text-center text-[11px] leading-relaxed text-content-faint">
        {t('chat.mascot.speakRepliesHint')}
      </p>
    </div>
  );
};

export default ChatMascotStage;
