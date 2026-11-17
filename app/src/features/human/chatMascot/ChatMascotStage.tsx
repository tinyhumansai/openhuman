import { useCallback } from 'react';

import { SettingsSwitch } from '../../../components/settings/controls';
import Button from '../../../components/ui/Button';
import { useT } from '../../../lib/i18n/I18nContext';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import {
  selectSpeakReplies,
  setChatMascotLiveVoicePhase,
  setSpeakReplies,
} from '../../../store/mascotSlice';
import LiveVoiceControls, { type LiveVoicePhase } from '../LiveVoiceControls';
import { useChatMascot } from './ChatMascotContext';

/**
 * The scaled-up mascot surface: the former Human page, folded into the chat's
 * right-hand column.
 *
 * The mascot itself is **not** rendered here — `ChatMascotOverlay` paints the
 * one shared instance over the `stageRef` placeholder. What lives here is the
 * voice interaction that only makes sense while expanded: the live voice-agent
 * session (`LiveVoiceControls`), the speak-replies switch, and the collapse
 * control.
 *
 * The chat's text composer stays live in the left column throughout, so the
 * user can type or talk without leaving this state.
 */
const ChatMascotStage = () => {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const { stageRef, collapse, consumeVoiceStart, liveAudioRef } = useChatMascot();
  const speakReplies = useAppSelector(selectSpeakReplies);
  // The live voice agent talks into the conversation that is open in the chat
  // column. Read once at session start; switching threads mid-call keeps the
  // call bound to the thread it started in.
  const selectedThreadId = useAppSelector(state => state.thread.selectedThreadId);

  // The overlay reads the phase from Redux for its listening pose, lip-sync gate
  // and to silence speak-replies TTS while the live agent owns the audio.
  const handlePhaseChange = useCallback(
    (phase: LiveVoicePhase) => {
      dispatch(setChatMascotLiveVoicePhase(phase));
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

      {/* The stage's voice mode is the live voice agent. Clicking the docked
          mascot asks for a session (`consumeVoiceStart`); collapsing unmounts
          this control, which ends the call. */}
      <LiveVoiceControls
        threadId={selectedThreadId}
        consumeAutoStart={consumeVoiceStart}
        audioRef={liveAudioRef}
        onPhaseChange={handlePhaseChange}
      />

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
