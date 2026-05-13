import { convertFileSrc } from '@tauri-apps/api/core';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import { type ChatSendError, chatSendError } from '../chat/chatSendError';
import { checkPromptInjection, promptGuardMessage } from '../chat/promptInjectionGuard';
import TokenUsagePill from '../components/chat/TokenUsagePill';
import { ConfirmationModal } from '../components/intelligence/ConfirmationModal';
import PillTabBar from '../components/PillTabBar';
import UpsellBanner from '../components/upsell/UpsellBanner';
import { dismissBanner, shouldShowBanner } from '../components/upsell/upsellDismissState';
import UsageLimitModal from '../components/upsell/UsageLimitModal';
import MicCloudComposer from '../features/human/MicCloudComposer';
// [#1123] Commented out — welcome-agent onboarding replaced by Joyride walkthrough
// import { ONBOARDING_WELCOME_THREAD_LABEL } from '../constants/onboardingChat';
import { useStickToBottom } from '../hooks/useStickToBottom';
import { useUsageState } from '../hooks/useUsageState';
import { trackEvent } from '../services/analytics';
// [#1123] getCoreStateSnapshot and isWelcomeLocked commented out — welcome-agent onboarding replaced by Joyride walkthrough
// import { getCoreStateSnapshot, isWelcomeLocked } from '../lib/coreState/store';
// [#1123] Commented out — welcome-agent onboarding replaced by Joyride walkthrough
// import { useCoreState } from '../providers/CoreStateProvider';
import { chatCancel, chatSend, useRustChat } from '../services/chatService';
import { store } from '../store';
import {
  beginInferenceTurn,
  clearRuntimeForThread,
  fetchAndHydrateTurnState,
  setToolTimelineForThread,
} from '../store/chatRuntimeSlice';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import { selectSocketStatus } from '../store/socketSelectors';
import {
  addMessageLocal,
  createNewThread,
  deleteThread,
  loadThreadMessages,
  loadThreads,
  persistReaction,
  setActiveThread,
  setSelectedThread,
  THREAD_NOT_FOUND_MESSAGE,
} from '../store/threadSlice';
import type { ConfirmationModal as ConfirmationModalType } from '../types/intelligence';
import type { ThreadMessage } from '../types/thread';
import { splitAgentMessageIntoBubbles } from '../utils/agentMessageBubbles';
import { BILLING_DASHBOARD_URL } from '../utils/links';
import { openUrl } from '../utils/openUrl';
import {
  isTauri,
  notifyOverlaySttState,
  openhumanAutocompleteAccept,
  openhumanAutocompleteCurrent,
  openhumanVoiceStatus,
  openhumanVoiceTranscribeBytes,
  openhumanVoiceTts,
} from '../utils/tauriCommands';
import { formatTimelineEntry } from '../utils/toolTimelineFormatting';
import { AgentMessageBubble, BubbleMarkdown } from './conversations/components/AgentMessageBubble';
import { CitationChips, type MessageCitation } from './conversations/components/CitationChips';
import { LimitPill } from './conversations/components/LimitPill';
import { ToolTimelineBlock } from './conversations/components/ToolTimelineBlock';
import {
  evaluateComposerSend,
  getComposerBlockedSendFeedback,
  handleComposerSlashCommand,
} from './conversations/composerSendDecision';
import {
  type AgentBubblePosition,
  buildAcceptedInlineCompletion,
  formatRelativeTime,
  formatResetTime,
  getInlineCompletionSuffix,
} from './conversations/utils/format';

// Chat uses the reasoning model; `agentic-v1` is reserved for sub-agents
// that execute tool calls, not the primary user-facing conversation.
const CHAT_MODEL_ID = 'reasoning-v1';
/** Maximum trailing characters rendered in the live-streaming assistant
 *  preview bubble. The full response is revealed via `addInferenceResponse`
 *  on `chat_done` — this is purely a ticker-tape affordance to signal
 *  progress without jumping the scroll position as tokens arrive. */
const STREAMING_PREVIEW_CHARS = 120;
type InputMode = 'text' | 'voice';
type ReplyMode = 'text' | 'voice';
const AUTOCOMPLETE_POLL_DEBOUNCE_MS = 320;
const AUTOCOMPLETE_MIN_CONTEXT_CHARS = 3;

interface ConversationsProps {
  /**
   * `page` (default) renders the centered max-w-2xl card layout used as
   * a top-level route at /conversations. `sidebar` drops the centering
   * and width cap so the panel can be embedded as a right rail inside
   * another page (e.g. /accounts).
   */
  variant?: 'page' | 'sidebar';
  /**
   * Composer mode. `text` (default) uses the textarea + send button.
   * `mic-cloud` swaps the entire composer for a single mic button that
   * captures audio via `MediaRecorder`, transcribes it through the cloud
   * STT proxy, then routes the transcript through the same send path.
   * Used by the mascot tab so the only interaction is voice.
   */
  composer?: 'text' | 'mic-cloud';
}

export function isComposerInteractionBlocked(args: {
  activeThreadId: string | null;
  welcomePending: boolean;
  rustChat: boolean;
}): boolean {
  return !args.rustChat || Boolean(args.activeThreadId) || args.welcomePending;
}

/**
 * Normalise the value thrown out of `dispatch(loadThreads()).unwrap()` into a
 * displayable string. `createAsyncThunk` re-throws Redux's `SerializedError`
 * (a plain object, not an `Error` instance) when the thunk rejects — which is
 * why the original Sentry report (OPENHUMAN-REACT-X) showed up as
 * "Non-Error promise rejection captured with value: …" rather than a stack.
 * Exported so the mount-effect's `.catch` stays a one-liner and the message
 * shape can be unit-tested without mounting the full page.
 */
export function formatThreadLoadError(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (err && typeof err === 'object' && 'message' in err) {
    const message = (err as { message?: unknown }).message;
    if (typeof message === 'string') return message;
  }
  return String(err);
}

// [#1123] Commented out — welcome-agent onboarding replaced by Joyride walkthrough
// function WelcomeThinkingTypewriter() {
//   const text = 'Your agent is thinking...';
//   const [visibleChars, setVisibleChars] = useState(0);
//
//   useEffect(() => {
//     const isComplete = visibleChars >= text.length;
//     const delayMs = isComplete ? 950 : 42;
//     const timeoutId = window.setTimeout(() => {
//       setVisibleChars(current => (current >= text.length ? 0 : current + 1));
//     }, delayMs);
//
//     return () => window.clearTimeout(timeoutId);
//   }, [text.length, visibleChars]);
//
//   return (
//     <p className="flex items-center text-sm text-stone-600 font-mono tracking-tight">
//       <span>{text.slice(0, visibleChars)}</span>
//       <span
//         aria-hidden="true"
//         className="ml-0.5 inline-block h-4 w-px bg-stone-400 animate-pulse"
//       />
//     </p>
//   );
// }

const Conversations = ({ variant = 'page', composer = 'text' }: ConversationsProps = {}) => {
  const dispatch = useAppDispatch();
  const navigate = useNavigate();
  const {
    threads,
    selectedThreadId,
    messages,
    isLoadingMessages,
    messagesError,
    activeThreadId,
    // [#1123] welcomeThreadId commented out — welcome-agent onboarding replaced by Joyride walkthrough
    // welcomeThreadId,
  } = useAppSelector(state => state.thread);

  // [#1123] Commented out — welcome-agent onboarding replaced by Joyride walkthrough
  // const { snapshot } = useCoreState();
  // const welcomeLocked = isWelcomeLocked(snapshot);

  // [#1123] Commented out — welcome-agent onboarding replaced by Joyride walkthrough
  // While the proactive welcome agent is running and hasn't published its
  // first message yet, hide the composer (and a few other non-message
  // chrome bits) so the user just sees the "Your agent is thinking..."
  // loader. Flips off the moment the first agent message arrives.
  // const welcomePending =
  //   !!welcomeThreadId && selectedThreadId === welcomeThreadId && messages.length === 0;
  // const chatOnboardingCompleted = snapshot.chatOnboardingCompleted;
  // const previousChatOnboardingCompletedRef = useRef<boolean | null>(null);
  // Guard against the mount-time `loadThreads()` promise resolving AFTER
  // the welcome-lock unlock transition creates a fresh thread. Without
  // this, the stale `.then(...)` would re-select the old welcome thread
  // and clobber the auto-created one (#883 CodeRabbit feedback).
  // const skipInitialThreadSelectionRef = useRef(false);

  const [showSidebar, setShowSidebar] = useState(true);
  const [inputValue, setInputValue] = useState('');
  const [copiedMessageId, setCopiedMessageId] = useState<string | null>(null);
  const [inputMode, setInputMode] = useState<InputMode>('text');
  const [replyMode, setReplyMode] = useState<ReplyMode>('text');
  const [isRecording, setIsRecording] = useState(false);
  const [isTranscribing, setIsTranscribing] = useState(false);
  const [voiceStatus, setVoiceStatus] = useState<string | null>(null);
  const [isPlayingReply, setIsPlayingReply] = useState(false);
  const [selectedLabel, setSelectedLabel] = useState<string>('all');
  const [inlineSuggestionValue, setInlineSuggestionValue] = useState('');
  const [sendError, setSendError] = useState<ChatSendError | null>(null);
  const [sendAdvisory, setSendAdvisory] = useState<string | null>(null);
  const socketStatus = useAppSelector(selectSocketStatus);
  const toolTimelineByThread = useAppSelector(state => state.chatRuntime.toolTimelineByThread);
  const inferenceStatusByThread = useAppSelector(
    state => state.chatRuntime.inferenceStatusByThread
  );
  const streamingAssistantByThread = useAppSelector(
    state => state.chatRuntime.streamingAssistantByThread
  );
  const inferenceTurnLifecycleByThread = useAppSelector(
    state => state.chatRuntime.inferenceTurnLifecycleByThread
  );
  const rustChat = useRustChat();
  const [reactionPickerMsgId, setReactionPickerMsgId] = useState<string | null>(null);

  const {
    teamUsage,
    isLoading: isLoadingBudget,
    isAtLimit,
    isBudgetExhausted,
    isRateLimited,
    isNearLimit,
    isFreeTier,
    shouldShowBudgetCompletedMessage,
    usagePct10h,
    usagePct7d,
    currentTier,
  } = useUsageState();
  const [showLimitModal, setShowLimitModal] = useState(false);
  const [deleteModal, setDeleteModal] = useState<ConfirmationModalType>({
    isOpen: false,
    title: '',
    message: '',
    onConfirm: () => {},
    onCancel: () => {},
  });

  const textInputRef = useRef<HTMLTextAreaElement>(null);
  const mediaRecorderRef = useRef<MediaRecorder | null>(null);
  const mediaStreamRef = useRef<MediaStream | null>(null);
  const audioChunksRef = useRef<Blob[]>([]);
  const replyAudioRef = useRef<HTMLAudioElement | null>(null);
  const lastSpokenMessageIdRef = useRef<string | null>(null);
  const autocompleteDebounceRef = useRef<number | null>(null);
  const autocompleteRequestSeqRef = useRef(0);
  const sendingTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Thread id whose send started the current silence timer. Tracked separately
  // from `selectedThreadId` so switching threads mid-turn doesn't move the
  // timer's reference point.
  const sendingThreadIdRef = useRef<string | null>(null);

  const getAudioExtension = (mimeType: string): string => {
    const lower = mimeType.toLowerCase();
    if (lower.includes('webm')) return 'webm';
    if (lower.includes('ogg')) return 'ogg';
    if (lower.includes('wav')) return 'wav';
    if (lower.includes('mp4') || lower.includes('mpeg') || lower.includes('aac')) return 'm4a';
    return 'webm';
  };
  const canUseMicrophoneApi =
    typeof navigator !== 'undefined' &&
    typeof navigator.mediaDevices !== 'undefined' &&
    typeof navigator.mediaDevices.getUserMedia === 'function';

  const handleCreateNewThread = async () => {
    const thread = await dispatch(createNewThread()).unwrap();
    dispatch(setSelectedThread(thread.id));
    void dispatch(loadThreadMessages(thread.id));
  };

  useEffect(() => {
    let cancelled = false;

    void dispatch(loadThreads())
      .unwrap()
      .then(data => {
        // [#1123] Commented out — welcome-agent onboarding replaced by Joyride walkthrough
        // if (cancelled || skipInitialThreadSelectionRef.current) return;
        if (cancelled) return;
        // [#1123] Commented out — welcome-agent onboarding replaced by Joyride walkthrough
        // Always prefer the welcome thread during lockdown regardless of
        // whether the server list is empty or not. Without this guard the
        // stale `.then` could select a pre-existing thread from a prior
        // session and pull the user out of the welcome conversation.
        // const snapForSelect = getCoreStateSnapshot().snapshot;
        // const threadStateForSelect = store.getState().thread;
        // if (isWelcomeLocked(snapForSelect) && threadStateForSelect.welcomeThreadId) {
        //   dispatch(setSelectedThread(threadStateForSelect.welcomeThreadId));
        //   void dispatch(loadThreadMessages(threadStateForSelect.welcomeThreadId));
        //   return;
        // }
        const threadStateForSelect = store.getState().thread;
        if (data.threads.length > 0) {
          // Prefer the thread the user was last viewing (persisted across
          // reloads via redux-persist on the `thread` slice). Only fall
          // through to "most recent" if that thread no longer exists
          // server-side (deleted, purged, or different user).
          const persistedId = threadStateForSelect.selectedThreadId;
          const resumeId =
            persistedId && data.threads.some(t => t.id === persistedId)
              ? persistedId
              : data.threads[0].id;
          dispatch(setSelectedThread(resumeId));
          void dispatch(loadThreadMessages(resumeId));
        } else {
          void handleCreateNewThread();
        }
      })
      .catch(err => {
        if (cancelled) return;
        console.warn('[conversations] loadThreads failed on mount:', formatThreadLoadError(err));
      });

    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dispatch]);

  useEffect(() => {
    if (selectedThreadId) {
      void dispatch(loadThreadMessages(selectedThreadId));
      void dispatch(fetchAndHydrateTurnState(selectedThreadId));
    }
  }, [selectedThreadId, dispatch]);

  // [#1123] Commented out — welcome-agent onboarding replaced by Joyride walkthrough
  // Welcome lockdown unlock (#883) — when `chatOnboardingCompleted`
  // transitions from `false` → `true` (the welcome agent just called
  // `complete_onboarding(action: "complete")`), open a fresh thread so
  // the user starts their first "real" conversation with the orchestrator
  // instead of continuing the welcome thread. Ref-tracked one-shot so
  // the 2s snapshot poll cannot re-fire this.
  // useEffect(() => {
  //   const prev = previousChatOnboardingCompletedRef.current;
  //   previousChatOnboardingCompletedRef.current = chatOnboardingCompleted;
  //   if (prev === false && chatOnboardingCompleted === true) {
  //     // Signal the mount-time `loadThreads()` promise to bail if it is
  //     // still pending — otherwise its stale resolution would overwrite
  //     // our freshly created thread selection.
  //     skipInitialThreadSelectionRef.current = true;
  //     console.debug('[welcome-lock] chat onboarding completed — opening new thread');
  //     void handleCreateNewThread();
  //   }
  //   // handleCreateNewThread is stable for the component lifetime (only
  //   // uses `dispatch`); the ref guards against duplicate fires.
  //   // eslint-disable-next-line react-hooks/exhaustive-deps
  // }, [chatOnboardingCompleted]);

  const location = useLocation();
  const { containerRef: messagesContainerRef, endRef: messagesEndRef } = useStickToBottom(
    messages,
    selectedThreadId,
    location.pathname
  );

  useEffect(() => {
    const onDictationInsert = (event: Event) => {
      const customEvent = event as CustomEvent<{ text?: string }>;
      const text = customEvent.detail?.text?.trim();
      if (!text) return;

      customEvent.preventDefault();
      setInputMode('text');
      setInputValue(prev => {
        const base = prev.trim();
        if (!base) return text;
        return `${base}${base.endsWith(' ') ? '' : ' '}${text}`;
      });

      window.requestAnimationFrame(() => {
        textInputRef.current?.focus();
      });
    };

    window.addEventListener('dictation://insert-text', onDictationInsert as EventListener);
    return () =>
      window.removeEventListener('dictation://insert-text', onDictationInsert as EventListener);
  }, []);

  useEffect(() => {
    if (sendError && inputValue.length > 0) {
      setSendError(null);
    }
    if (sendAdvisory && inputValue.length > 0) {
      setSendAdvisory(null);
    }
  }, [inputValue, sendAdvisory, sendError]);

  const armSilenceTimer = (threadId: string) => {
    if (sendingTimeoutRef.current) clearTimeout(sendingTimeoutRef.current);
    sendingThreadIdRef.current = threadId;
    sendingTimeoutRef.current = setTimeout(() => {
      console.warn('[chat] silence timeout: no inference signal for 120s');
      setSendError(
        chatSendError(
          'safety_timeout',
          'No response from the agent after 2 minutes. Try again or check your connection.'
        )
      );
      dispatch(clearRuntimeForThread({ threadId }));
      dispatch(setActiveThread(null));
      sendingTimeoutRef.current = null;
      sendingThreadIdRef.current = null;
    }, 120_000);
  };

  // Rearm the silence timer on every inference signal for the sending
  // thread. Tool / iteration / subagent events bump `inferenceStatusByThread`;
  // pure-text streams (no tools) only bump `streamingAssistantByThread`, so
  // both must be watched — otherwise a long text stream would trip the
  // safety timer mid-reply. When the status is cleared (chat_done /
  // chat_error), drop the timer — the completion handlers own UI cleanup.
  useEffect(() => {
    const threadId = sendingThreadIdRef.current;
    if (!threadId || !sendingTimeoutRef.current) return;
    const status = inferenceStatusByThread[threadId];
    if (status === undefined) {
      clearTimeout(sendingTimeoutRef.current);
      sendingTimeoutRef.current = null;
      sendingThreadIdRef.current = null;
      return;
    }
    armSilenceTimer(threadId);
    // armSilenceTimer is stable (refs + dispatch); depending on the
    // selector references is enough to rearm on every progress event.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [inferenceStatusByThread, streamingAssistantByThread]);

  useEffect(() => {
    if (
      !isTauri() ||
      !rustChat ||
      inputMode !== 'text' ||
      Boolean(activeThreadId) ||
      inputValue.trim().length < AUTOCOMPLETE_MIN_CONTEXT_CHARS
    ) {
      setInlineSuggestionValue('');
      return;
    }

    if (autocompleteDebounceRef.current !== null) {
      window.clearTimeout(autocompleteDebounceRef.current);
    }

    autocompleteDebounceRef.current = window.setTimeout(() => {
      const requestSeq = autocompleteRequestSeqRef.current + 1;
      autocompleteRequestSeqRef.current = requestSeq;

      void openhumanAutocompleteCurrent({ context: inputValue })
        .then(response => {
          if (autocompleteRequestSeqRef.current !== requestSeq) return;
          setInlineSuggestionValue(response.result.suggestion?.value ?? '');
        })
        .catch(() => {
          if (autocompleteRequestSeqRef.current !== requestSeq) return;
          setInlineSuggestionValue('');
        });
    }, AUTOCOMPLETE_POLL_DEBOUNCE_MS);

    return () => {
      if (autocompleteDebounceRef.current !== null) {
        window.clearTimeout(autocompleteDebounceRef.current);
        autocompleteDebounceRef.current = null;
      }
    };
  }, [activeThreadId, inputValue, inputMode, rustChat]);

  useEffect(() => {
    return () => {
      mediaRecorderRef.current?.stop();
      mediaStreamRef.current?.getTracks().forEach(track => track.stop());
      replyAudioRef.current?.pause();
      replyAudioRef.current = null;
    };
  }, []);

  useEffect(() => {
    if (inputMode === 'text' && isRecording) {
      mediaRecorderRef.current?.stop();
    }
  }, [inputMode, isRecording]);

  useEffect(() => {
    if (inputMode === 'voice') {
      setReplyMode('voice');
    } else if (replyMode === 'voice') {
      setReplyMode('text');
    }
  }, [inputMode, replyMode]);

  // Proactively check voice binary availability when switching to voice mode
  useEffect(() => {
    if (inputMode !== 'voice' || !rustChat) return;
    let cancelled = false;
    void (async () => {
      try {
        const status = await openhumanVoiceStatus();
        if (cancelled) return;
        if (!status.stt_available) {
          setVoiceStatus(
            'Voice input needs a speech model to work. Go to Settings > Local AI Models to set it up.'
          );
        } else {
          setVoiceStatus('Ready — tap "Start Talking" to record.');
        }
      } catch {
        if (!cancelled) {
          setVoiceStatus('Could not check voice availability.');
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [inputMode, rustChat]);

  const handleSlashCommand = (command: string): boolean => {
    const decision = handleComposerSlashCommand(command, false);
    if (decision.kind === 'not_handled') return false;

    setInputValue('');
    void handleCreateNewThread();
    return true;
  };

  const handleSendMessage = async (text?: string) => {
    const normalized = text ?? inputValue;
    const trimmedInput = normalized.trim();

    if (handleSlashCommand(trimmedInput)) return;

    const sendDecision = evaluateComposerSend({
      rawText: normalized,
      selectedThreadId,
      composerInteractionBlocked,
      isAtLimit,
      socketStatus,
    });
    const trimmed = sendDecision.trimmedText;

    if (
      sendDecision.blockReason === 'empty_input' ||
      sendDecision.blockReason === 'missing_thread' ||
      sendDecision.blockReason === 'composer_blocked'
    ) {
      return;
    }

    const promptGuard = checkPromptInjection(trimmed);
    if (promptGuard.verdict === 'review' || promptGuard.verdict === 'block') {
      setSendAdvisory(promptGuardMessage(promptGuard));
    } else {
      setSendAdvisory(null);
    }

    if (!sendDecision.shouldSend) {
      const blockedFeedback = getComposerBlockedSendFeedback(sendDecision.blockReason);
      if (blockedFeedback?.showLimitModal) {
        setShowLimitModal(true);
      }
      if (blockedFeedback) {
        setSendError(chatSendError(blockedFeedback.error.code, blockedFeedback.error.message));
      }
      return;
    }

    const sendingThreadId = selectedThreadId;
    if (!sendingThreadId) return;
    const userMessage: ThreadMessage = {
      id: `msg_${globalThis.crypto.randomUUID()}`,
      content: trimmed,
      type: 'text',
      extraMetadata: {},
      sender: 'user',
      createdAt: new Date().toISOString(),
    };

    try {
      await dispatch(addMessageLocal({ threadId: sendingThreadId, message: userMessage })).unwrap();
    } catch (error) {
      // RTK's unwrap() re-throws the rejectWithValue payload directly (a plain
      // string, not an Error). Check for the stale-thread sentinel before
      // coercing to a display string so this guard doesn't accidentally match
      // unrelated errors whose `.toString()` happens to equal the sentinel.
      if (error === THREAD_NOT_FOUND_MESSAGE) {
        setSendError(null);
        return;
      }
      const msg = error instanceof Error ? error.message : String(error);
      setSendError(chatSendError('cloud_send_failed', msg));
      return;
    }
    setInputValue('');
    setSendError(null);
    // Silence timer: fires only if 600s pass without ANY inference progress
    // (tool call, tool result, iteration start, subagent event, text delta).
    // The effect below rearms this timer whenever `inferenceStatusByThread`
    // changes for `sendingThreadId`, so long-running agent turns stay alive
    // as long as the backend is emitting signals. A truly hung server still
    // fails fast.
    armSilenceTimer(sendingThreadId);
    dispatch(setToolTimelineForThread({ threadId: sendingThreadId, entries: [] }));
    dispatch(beginInferenceTurn({ threadId: sendingThreadId }));
    dispatch(setActiveThread(sendingThreadId));

    // ── Cloud socket path ─────────────────────────────────────────────────────
    // Always route primary chat through the cloud backend via socket.
    // Local model (Ollama) is used only for supplementary features
    // (auto-react, autocomplete, etc.) — never as a primary chat path.
    try {
      await chatSend({ threadId: sendingThreadId, message: trimmed, model: CHAT_MODEL_ID });
      trackEvent('chat_message_sent');

      // Active-thread reset happens in the global ChatRuntimeProvider events.
    } catch (err) {
      // Chat loop errors are emitted via socket events; this catch handles emit-level failures.
      if (sendingTimeoutRef.current) {
        clearTimeout(sendingTimeoutRef.current);
        sendingTimeoutRef.current = null;
      }
      sendingThreadIdRef.current = null;
      const msg = err instanceof Error ? err.message : String(err);
      if (
        msg.toLowerCase().includes('blocked by a security policy') ||
        msg.toLowerCase().includes('flagged for security review')
      ) {
        const code = msg.toLowerCase().includes('flagged for security review')
          ? 'prompt_review'
          : 'prompt_blocked';
        setSendError(chatSendError(code, msg));
      } else {
        setSendError(chatSendError('cloud_send_failed', msg));
      }
      dispatch(clearRuntimeForThread({ threadId: sendingThreadId }));
      dispatch(setActiveThread(null));
    }
  };

  const transcribeAndSendAudio = async (mimeType: string) => {
    setIsRecording(false);
    mediaRecorderRef.current = null;
    mediaStreamRef.current?.getTracks().forEach(track => track.stop());
    mediaStreamRef.current = null;

    const chunks = audioChunksRef.current;
    audioChunksRef.current = [];
    if (chunks.length === 0) {
      notifyOverlaySttState('cancelled');
      setVoiceStatus('No audio captured. Try again.');
      return;
    }

    setIsTranscribing(true);
    setVoiceStatus('Transcribing with Whisper…');
    try {
      const blob = new Blob(chunks, { type: mimeType || 'audio/webm' });
      const audioBytes = Array.from(new Uint8Array(await blob.arrayBuffer()));
      const extension = getAudioExtension(mimeType || blob.type);

      // Build conversation context from recent messages for LLM cleanup.
      const recentMessages = messages.slice(-10);
      const context =
        recentMessages.length > 0
          ? recentMessages.map(m => `${m.sender}: ${m.content}`).join('\n')
          : undefined;

      const result = await openhumanVoiceTranscribeBytes(audioBytes, extension, context);
      const transcript = result.text.trim();

      if (!transcript) {
        notifyOverlaySttState('cancelled');
        setVoiceStatus('No speech detected. Try again.');
        return;
      }

      notifyOverlaySttState('transcription_done', transcript);
      setVoiceStatus(`Heard: ${transcript}`);
      await handleSendMessage(transcript);
    } catch (err) {
      notifyOverlaySttState('error');
      const message = err instanceof Error ? err.message : String(err);
      const isSetupIssue =
        message.includes('whisper') ||
        message.includes('binary not found') ||
        message.includes('STT model');
      setSendError(
        chatSendError(
          isSetupIssue ? 'stt_not_ready' : 'voice_transcription',
          isSetupIssue
            ? 'Voice input needs a speech model. Go to Settings to download one.'
            : `Voice transcription failed: ${message}`
        )
      );
      setVoiceStatus(null);
    } finally {
      setIsTranscribing(false);
    }
  };

  const handleVoiceRecordToggle = async () => {
    if (!rustChat || Boolean(activeThreadId) || isTranscribing) return;
    if (!canUseMicrophoneApi) {
      setSendError(
        chatSendError(
          'microphone_unavailable',
          'Microphone capture is unavailable in this runtime. Use Text mode, or run the desktop app bundle with microphone permissions enabled.'
        )
      );
      return;
    }

    if (isRecording) {
      mediaRecorderRef.current?.stop();
      return;
    }

    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      mediaStreamRef.current = stream;

      const preferredTypes = [
        'audio/webm;codecs=opus',
        'audio/webm',
        'audio/ogg;codecs=opus',
        'audio/ogg',
        'audio/mp4',
      ];
      const supportedType = preferredTypes.find(type => MediaRecorder.isTypeSupported(type));
      const recorder = supportedType
        ? new MediaRecorder(stream, { mimeType: supportedType })
        : new MediaRecorder(stream);

      audioChunksRef.current = [];
      recorder.ondataavailable = event => {
        if (event.data.size > 0) {
          audioChunksRef.current.push(event.data);
        }
      };
      recorder.onerror = () => {
        notifyOverlaySttState('error');
        setIsRecording(false);
        mediaStreamRef.current?.getTracks().forEach(track => track.stop());
        mediaStreamRef.current = null;
        setSendError(chatSendError('microphone_recording', 'Microphone recording failed.'));
      };
      recorder.onstop = () => {
        void transcribeAndSendAudio(recorder.mimeType);
      };

      mediaRecorderRef.current = recorder;
      setVoiceStatus('Listening… click Stop to send.');
      setSendError(null);
      setIsRecording(true);
      recorder.start();
      notifyOverlaySttState('recording_started');
    } catch (err) {
      notifyOverlaySttState('error');
      const message = err instanceof Error ? err.message : String(err);
      setSendError(chatSendError('microphone_access', `Microphone access failed: ${message}`));
      setVoiceStatus(null);
    }
  };

  useEffect(() => {
    const latestAgentMessage = [...messages].reverse().find(m => m.sender === 'agent');
    if (!latestAgentMessage) return;

    if (replyMode === 'text') {
      lastSpokenMessageIdRef.current = latestAgentMessage.id;
      replyAudioRef.current?.pause();
      replyAudioRef.current = null;
      setIsPlayingReply(false);
      return;
    }

    if (!rustChat || latestAgentMessage.id === lastSpokenMessageIdRef.current) return;

    lastSpokenMessageIdRef.current = latestAgentMessage.id;
    let cancelled = false;
    setIsPlayingReply(true);

    void (async () => {
      try {
        const ttsResult = await openhumanVoiceTts(latestAgentMessage.content);
        if (cancelled) return;

        const audioSrc = convertFileSrc(ttsResult.output_path);
        const audio = new window.Audio(audioSrc);
        replyAudioRef.current?.pause();
        replyAudioRef.current = audio;

        await audio.play();
      } catch {
        if (!cancelled) {
          setSendError(chatSendError('voice_playback', 'Failed to play voice reply.'));
        }
      } finally {
        if (!cancelled) {
          setIsPlayingReply(false);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [messages, replyMode, rustChat]);

  const handleInputKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    const inlineSuffix = getInlineCompletionSuffix(inputValue, inlineSuggestionValue);
    const textarea = e.currentTarget;
    const caretAtEnd =
      textarea.selectionStart === inputValue.length && textarea.selectionEnd === inputValue.length;
    const tryAcceptInlineSuggestion = () => {
      const nextValue = buildAcceptedInlineCompletion(inputValue, inlineSuffix);
      if (!nextValue || nextValue === inputValue) return false;
      setInputValue(nextValue);
      setInlineSuggestionValue('');
      if (isTauri()) {
        void openhumanAutocompleteAccept({ suggestion: nextValue, skip_apply: true }).catch(() => {
          // Keep local UX smooth even if accept RPC fails.
        });
      }
      return true;
    };

    if (
      e.key === 'Tab' &&
      !e.shiftKey &&
      !e.altKey &&
      !e.ctrlKey &&
      !e.metaKey &&
      inlineSuffix.length > 0 &&
      caretAtEnd
    ) {
      e.preventDefault();
      tryAcceptInlineSuggestion();
      return;
    }

    if (e.key === 'ArrowRight' && inlineSuffix.length > 0 && caretAtEnd) {
      e.preventDefault();
      tryAcceptInlineSuggestion();
      return;
    }

    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void handleSendMessage();
    }
  };

  const handleCopyMessage = async (messageId: string, content: string) => {
    try {
      await navigator.clipboard.writeText(content);
      setCopiedMessageId(messageId);
      setTimeout(() => setCopiedMessageId(null), 1500);
    } catch {
      // Clipboard API not available — silently fail
    }
  };

  const selectedThreadToolTimeline = selectedThreadId
    ? (toolTimelineByThread[selectedThreadId] ?? [])
    : [];
  const visibleMessages = messages.filter(msg => !msg.extraMetadata?.hidden);
  const hasVisibleMessages = visibleMessages.length > 0;
  const latestVisibleMessage = visibleMessages[visibleMessages.length - 1] ?? null;
  const latestVisibleAgentMessage = [...visibleMessages]
    .reverse()
    .find(msg => msg.sender === 'agent');
  const activeSubagentTimelineEntry = selectedThreadToolTimeline.find(
    entry => entry.status === 'running' && entry.name.startsWith('subagent:')
  );
  const activeToolTimelineEntry = [...selectedThreadToolTimeline]
    .reverse()
    .find(entry => entry.status === 'running' && !entry.name.startsWith('subagent:'));
  const selectedInferenceStatus = selectedThreadId
    ? (inferenceStatusByThread[selectedThreadId] ?? null)
    : null;
  const selectedStreamingAssistant = selectedThreadId
    ? (streamingAssistantByThread[selectedThreadId] ?? null)
    : null;
  const inlineCompletionSuffix = getInlineCompletionSuffix(inputValue, inlineSuggestionValue);
  // Blocks all composer interaction while a turn is in-flight, the
  // proactive welcome opener is pending, or Rust chat is unavailable.
  // isSending: the *selected* thread is in-flight (drives selected-thread UI only).
  // [#1123] welcomePending removed — welcome-agent onboarding replaced by Joyride walkthrough
  const composerInteractionBlocked = isComposerInteractionBlocked({
    activeThreadId,
    welcomePending: false,
    rustChat,
  });
  // Auto-focus the composer when a thread becomes selected and the composer
  // isn't blocked. Without this, navigating into a thread from elsewhere in
  // the app (e.g. acting on a subconscious reflection in the Intelligence
  // tab — `IntelligenceSubconsciousTab.handleNavigateToReflectionThread`
  // dispatches `setSelectedThread` then routes to `/chat`) leaves focus on
  // the unmounted source button, falling back to `document.body`. The
  // textarea is rendered and enabled but ignores keystrokes until the user
  // clicks into it. Skip when there is no thread, when the composer is
  // disabled, when in voice mode, and when the user has focus on another
  // input/textarea/contenteditable (don't steal focus from a settings pane
  // the user just clicked into).
  useEffect(() => {
    if (!selectedThreadId) return;
    if (composerInteractionBlocked) return;
    if (inputMode !== 'text') return;
    const ta = textInputRef.current;
    if (!ta) return;
    const active = document.activeElement;
    if (
      active &&
      active !== document.body &&
      active !== ta &&
      (active.tagName === 'INPUT' ||
        active.tagName === 'TEXTAREA' ||
        active.getAttribute('contenteditable') === 'true')
    ) {
      return;
    }
    // rAF — wait for the textarea to be in the layout tree (selectedThread
    // changes can arrive a tick before the panel mounts on first navigation).
    const id = window.requestAnimationFrame(() => {
      textInputRef.current?.focus();
    });
    return () => window.cancelAnimationFrame(id);
  }, [selectedThreadId, composerInteractionBlocked, inputMode]);
  const isSending = Boolean(
    selectedThreadId &&
    (inferenceTurnLifecycleByThread[selectedThreadId] === 'started' ||
      inferenceTurnLifecycleByThread[selectedThreadId] === 'streaming')
  );
  const shouldRenderTimelineBeforeLatestAgentMessage =
    selectedThreadToolTimeline.length > 0 && !isSending && Boolean(latestVisibleAgentMessage);

  const filteredThreads = useMemo(() => {
    const base = threads.filter(t => {
      if (selectedLabel === 'all') return true;
      return t.labels?.includes(selectedLabel);
    });
    // [#1123] Commented out — welcome-agent onboarding replaced by Joyride walkthrough
    // if (!welcomeLocked) return base;
    // // During welcome lockdown only the onboarding welcome thread should
    // // appear — not stray blank threads from races or proactive:* handling.
    // if (welcomeThreadId) {
    //   return base.filter(t => t.id === welcomeThreadId);
    // }
    // // Fallback: welcomeThreadId not yet set but the server already returned the
    // // thread (e.g. hot-reload). Keep only onboarding-labelled threads so the
    // // welcome thread is visible rather than hidden behind the empty-state message.
    // return base.filter(t => (t.labels ?? []).includes(ONBOARDING_WELCOME_THREAD_LABEL));
    return base;
  }, [threads, selectedLabel]);

  const sortedThreads = useMemo(() => {
    return [...filteredThreads].sort(
      (a, b) => new Date(b.lastMessageAt).getTime() - new Date(a.lastMessageAt).getTime()
    );
  }, [filteredThreads]);

  const allLabels = useMemo(() => {
    return Array.from(new Set(threads.flatMap(t => t.labels ?? []))).sort();
  }, [threads]);

  // Fixed tab set so categories don't disappear when empty and the active
  // filter state remains unambiguous regardless of what threads exist.
  const labelTabs = [
    { label: 'All', value: 'all' },
    { label: 'Work', value: 'work' },
    { label: 'Briefing', value: 'briefing' },
    { label: 'Notification', value: 'notification' },
  ];

  // Reset stale selectedLabel when the last thread carrying that label is deleted.
  useEffect(() => {
    if (selectedLabel !== 'all' && !allLabels.includes(selectedLabel)) {
      setSelectedLabel('all');
    }
  }, [allLabels, selectedLabel]);

  const isSidebar = variant === 'sidebar';
  // [#1123] Commented out — welcome-agent onboarding replaced by Joyride walkthrough
  // During welcome lockdown keep the sidebar forced open so the user always
  // sees the single onboarding thread entry and cannot accidentally close the
  // panel via the toggle (leaving themselves with no thread list).
  // const effectiveShowSidebar = welcomeLocked ? true : showSidebar;
  const effectiveShowSidebar = showSidebar;

  // Stable title resolver used by both the sidebar thread list and the header.
  // [#1123] welcome-lock title override removed — Joyride walkthrough replaced welcome-agent
  const resolveThreadDisplayTitle = (threadId: string | null): string => {
    if (!threadId) return 'Select a thread';
    const t = threads.find(thr => thr.id === threadId);
    // [#1123] Commented out — welcome-agent onboarding replaced by Joyride walkthrough
    // if (
    //   welcomeLocked &&
    //   t?.id === welcomeThreadId &&
    //   (t?.labels ?? []).includes(ONBOARDING_WELCOME_THREAD_LABEL)
    // ) {
    //   return 'Onboarding';
    // }
    return t?.title ?? 'Select a thread';
  };

  return (
    <div
      className={
        isSidebar
          ? 'h-full relative z-10 flex overflow-hidden'
          : 'h-full relative z-10 flex justify-center overflow-hidden p-4 pt-6 gap-3'
      }>
      {/* Thread sidebar — only shown in page mode (when Conversations itself
          is a top-level route, not embedded as a sidebar in another page).
          During welcome lockdown the sidebar is always open (effectiveShowSidebar
          is clamped to true) so the single onboarding thread is always visible. */}
      {!isSidebar && effectiveShowSidebar && (
        <div className="w-64 flex-shrink-0 flex flex-col bg-white rounded-2xl shadow-soft border border-stone-200 overflow-hidden">
          <div className="flex items-center justify-between px-4 py-3 border-b border-stone-100">
            <h2 className="text-sm font-semibold text-stone-700">Threads</h2>
            {/* [#1123] welcomeLocked guard removed — always show new thread button */}
            <button
              onClick={() => void handleCreateNewThread()}
              className="w-7 h-7 flex items-center justify-center rounded-lg hover:bg-stone-100 text-stone-500 hover:text-stone-700 transition-colors"
              title="New thread">
              <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M12 4v16m8-8H4"
                />
              </svg>
            </button>
          </div>
          {/* [#1123] welcomeLocked guard removed — always show label filter */}
          <div className="px-4 py-2 border-b border-stone-50">
            <PillTabBar
              items={labelTabs}
              selected={selectedLabel}
              onChange={setSelectedLabel}
              containerClassName="flex gap-1 overflow-x-auto py-1 scrollbar-hide"
            />
          </div>
          <div className="flex-1 overflow-y-auto">
            {sortedThreads.length === 0 ? (
              <p className="px-4 py-6 text-xs text-stone-400 text-center">
                {selectedLabel === 'all' ? 'No threads yet' : `No "${selectedLabel}" threads`}
              </p>
            ) : (
              sortedThreads.map(thread => (
                <div
                  key={thread.id}
                  role="button"
                  tabIndex={0}
                  onClick={() => {
                    dispatch(setSelectedThread(thread.id));
                    void dispatch(loadThreadMessages(thread.id));
                  }}
                  onKeyDown={e => {
                    if (e.target !== e.currentTarget) return;
                    if (e.key === 'Enter' || e.key === ' ') {
                      e.preventDefault();
                      dispatch(setSelectedThread(thread.id));
                      void dispatch(loadThreadMessages(thread.id));
                    }
                  }}
                  className={`w-full text-left px-4 py-3 border-b border-stone-50 transition-colors group cursor-pointer ${
                    selectedThreadId === thread.id
                      ? 'bg-primary-50 border-l-2 border-l-primary-500'
                      : 'hover:bg-stone-50'
                  }`}>
                  <div className="flex items-center justify-between">
                    <p
                      className={`text-sm truncate flex-1 ${
                        selectedThreadId === thread.id
                          ? 'font-medium text-primary-700'
                          : 'text-stone-700'
                      }`}>
                      {resolveThreadDisplayTitle(thread.id)}
                    </p>
                    {/* [#1123] welcomeLocked guard removed — always show delete button */}
                    <button
                      onClick={e => {
                        e.stopPropagation();
                        setDeleteModal({
                          isOpen: true,
                          title: 'Delete thread',
                          message: `Are you sure you want to delete "${thread.title || 'Untitled thread'}"? This cannot be undone.`,
                          confirmText: 'Delete',
                          cancelText: 'Cancel',
                          destructive: true,
                          onConfirm: () => {
                            void dispatch(deleteThread(thread.id));
                          },
                          onCancel: () => {},
                        });
                      }}
                      className="ml-2 p-1 rounded opacity-0 group-hover:opacity-100 hover:bg-stone-200 text-stone-400 hover:text-coral-500 transition-all flex-shrink-0"
                      title="Delete thread">
                      <svg
                        className="w-3 h-3"
                        fill="none"
                        stroke="currentColor"
                        viewBox="0 0 24 24">
                        <path
                          strokeLinecap="round"
                          strokeLinejoin="round"
                          strokeWidth={2}
                          d="M6 18L18 6M6 6l12 12"
                        />
                      </svg>
                    </button>
                  </div>
                  {/* <div className="flex items-center gap-2 mt-0.5">
                    <span className="text-[10px] text-stone-400">
                      {formatRelativeTime(thread.lastMessageAt)}
                    </span>
                    {thread.messageCount > 0 && (
                      <span className="text-[10px] text-stone-400">
                        {thread.messageCount} msg{thread.messageCount !== 1 ? 's' : ''}
                      </span>
                    )}
                  </div> */}
                </div>
              ))
            )}
          </div>
        </div>
      )}

      {/* Main chat area */}
      <div
        className={
          isSidebar
            ? 'flex-1 flex flex-col min-w-0 bg-white border-l border-stone-200 overflow-hidden'
            : 'flex-1 flex flex-col min-w-0 max-w-2xl bg-white rounded-2xl shadow-soft border border-stone-200 overflow-hidden'
        }>
        {/* Chat header — only shown in page mode; the sidebar embed uses the
            parent page's chrome instead. Hidden entirely during welcome
            lockdown (#883) so the onboarding chat is just the conversation
            with no chrome around it. */}
        {!isSidebar && (
          <div
            className="flex items-center gap-2 px-4 py-2.5 border-b border-stone-100"
            data-walkthrough="chat-agent-panel">
            <button
              onClick={() => setShowSidebar(prev => !prev)}
              className="w-7 h-7 flex items-center justify-center rounded-lg hover:bg-stone-100 text-stone-500 hover:text-stone-700 transition-colors"
              title={effectiveShowSidebar ? 'Hide sidebar' : 'Show sidebar'}>
              <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M4 6h16M4 12h16M4 18h16"
                />
              </svg>
            </button>
            <h3 className="text-sm font-medium text-stone-700 truncate flex-1">
              {resolveThreadDisplayTitle(selectedThreadId)}
            </h3>
            {/* [#1123] welcomeLocked guard removed — always show token usage + new thread button */}
            <>
              <TokenUsagePill />
              <button
                onClick={() => void handleCreateNewThread()}
                className="px-2.5 py-1 rounded-lg text-xs font-medium text-primary-600 hover:bg-primary-50 transition-colors"
                title="New thread (/new)">
                + New
              </button>
            </>
          </div>
        )}
        <div ref={messagesContainerRef} className="flex-1 overflow-y-auto px-5 py-4 bg-[#f6f6f6]">
          {isLoadingMessages ? (
            <div className="space-y-4">
              {Array.from({ length: 4 }).map((_, i) => (
                <div key={i} className={`flex ${i % 2 === 0 ? 'justify-start' : 'justify-end'}`}>
                  <div
                    className={`h-12 rounded-2xl animate-pulse bg-stone-100 ${
                      i % 2 === 0 ? 'w-2/3' : 'w-1/2'
                    }`}
                  />
                </div>
              ))}
            </div>
          ) : messagesError ? (
            <div className="flex-1 flex flex-col items-center justify-center h-full">
              <svg
                className="w-8 h-8 text-coral-500/70 mb-3"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={1.5}
                  d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"
                />
              </svg>
              <p className="text-sm text-stone-400 mb-1">Failed to load messages</p>
              <p className="text-xs text-stone-600 mb-3 text-center">{messagesError}</p>
              <button
                onClick={() => window.location.reload()}
                className="text-xs text-primary-400 hover:text-primary-300 transition-colors">
                Reload
              </button>
            </div>
          ) : hasVisibleMessages ? (
            <div className="space-y-3">
              {visibleMessages.map(msg => (
                <div key={msg.id}>
                  {shouldRenderTimelineBeforeLatestAgentMessage &&
                    latestVisibleAgentMessage?.id === msg.id && (
                      <ToolTimelineBlock entries={selectedThreadToolTimeline} />
                    )}
                  <div
                    className={`group/msg flex ${msg.sender === 'user' ? 'justify-end' : 'justify-start'}`}>
                    <div className="relative w-fit max-w-[75%]">
                      {msg.sender === 'agent' ? (
                        <div className="space-y-1">
                          {splitAgentMessageIntoBubbles(msg.content).map(
                            (segment, index, parts) => {
                              const position: AgentBubblePosition =
                                parts.length === 1
                                  ? 'single'
                                  : index === 0
                                    ? 'first'
                                    : index === parts.length - 1
                                      ? 'last'
                                      : 'middle';

                              return (
                                <AgentMessageBubble
                                  key={`${msg.id}:${index}`}
                                  content={segment}
                                  position={position}
                                />
                              );
                            }
                          )}
                          {(() => {
                            const raw = msg.extraMetadata?.citations;
                            if (!Array.isArray(raw)) return null;
                            const citations = raw.filter(
                              (item): item is MessageCitation =>
                                typeof item === 'object' &&
                                item !== null &&
                                typeof (item as MessageCitation).id === 'string' &&
                                typeof (item as MessageCitation).key === 'string' &&
                                typeof (item as MessageCitation).snippet === 'string' &&
                                typeof (item as MessageCitation).timestamp === 'string'
                            );
                            if (citations.length === 0) return null;
                            return <CitationChips citations={citations} />;
                          })()}
                          {latestVisibleMessage?.id === msg.id && (
                            <p className="px-1 text-[10px] text-stone-400">
                              {formatRelativeTime(msg.createdAt)}
                            </p>
                          )}
                        </div>
                      ) : (
                        <div className="rounded-2xl px-4 py-2.5 bg-primary-500 text-white rounded-br-md break-words overflow-hidden">
                          <BubbleMarkdown content={msg.content} tone="user" />
                          {latestVisibleMessage?.id === msg.id && (
                            <p className="mt-1 text-[10px] text-white/60">
                              {formatRelativeTime(msg.createdAt)}
                            </p>
                          )}
                        </div>
                      )}
                      <button
                        onClick={() => handleCopyMessage(msg.id, msg.content)}
                        className={`absolute -top-1 ${msg.sender === 'user' ? '-left-8' : '-right-8'} p-1 rounded-md opacity-0 group-hover/msg:opacity-100 hover:bg-stone-100 text-stone-400 hover:text-stone-600 transition-all`}
                        title="Copy message">
                        {copiedMessageId === msg.id ? (
                          <svg
                            className="w-3.5 h-3.5 text-sage-500"
                            fill="none"
                            stroke="currentColor"
                            viewBox="0 0 24 24">
                            <path
                              strokeLinecap="round"
                              strokeLinejoin="round"
                              strokeWidth={2}
                              d="M5 13l4 4L19 7"
                            />
                          </svg>
                        ) : (
                          <svg
                            className="w-3.5 h-3.5"
                            fill="none"
                            stroke="currentColor"
                            viewBox="0 0 24 24">
                            <path
                              strokeLinecap="round"
                              strokeLinejoin="round"
                              strokeWidth={2}
                              d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z"
                            />
                          </svg>
                        )}
                      </button>
                      {(() => {
                        if (latestVisibleMessage?.id !== msg.id) return null;
                        const myReactions =
                          (msg.extraMetadata?.myReactions as string[] | undefined) ?? [];
                        const hasReactions = myReactions.length > 0;
                        // Show reaction row only for the most recent visible message.
                        if (!hasReactions && msg.sender !== 'agent') return null;
                        return (
                          <div className="mt-1 flex items-center gap-1 flex-wrap min-h-[20px]">
                            {myReactions.map(emoji => (
                              <button
                                key={emoji}
                                onClick={() =>
                                  selectedThreadId &&
                                  void dispatch(
                                    persistReaction({
                                      threadId: selectedThreadId,
                                      messageId: msg.id,
                                      emoji,
                                    })
                                  )
                                }
                                className="flex items-center gap-0.5 px-1.5 py-0.5 rounded-full bg-primary-100 border border-primary-200 text-xs transition-colors hover:bg-primary-200"
                                title={`Remove ${emoji}`}>
                                {emoji}
                              </button>
                            ))}
                            {msg.sender === 'agent' &&
                              (reactionPickerMsgId === msg.id ? (
                                <div className="flex items-center gap-0.5 px-1 py-0.5 rounded-full bg-stone-100">
                                  {['👍', '❤️', '😂', '🔥', '👀', '🎯'].map(emoji => (
                                    <button
                                      key={emoji}
                                      onClick={() => {
                                        if (selectedThreadId) {
                                          void dispatch(
                                            persistReaction({
                                              threadId: selectedThreadId,
                                              messageId: msg.id,
                                              emoji,
                                            })
                                          );
                                        }
                                        setReactionPickerMsgId(null);
                                      }}
                                      className="px-0.5 rounded text-sm hover:scale-125 transition-transform"
                                      title={emoji}>
                                      {emoji}
                                    </button>
                                  ))}
                                  <button
                                    onClick={() => setReactionPickerMsgId(null)}
                                    className="ml-0.5 text-stone-600 hover:text-stone-400 text-xs px-0.5">
                                    ✕
                                  </button>
                                </div>
                              ) : (
                                <button
                                  onClick={() => setReactionPickerMsgId(msg.id)}
                                  className="opacity-0 group-hover/msg:opacity-100 flex items-center px-1.5 py-0.5 rounded-full bg-stone-50 hover:bg-stone-200 text-stone-500 hover:text-stone-300 text-xs transition-all"
                                  title="Add reaction">
                                  +
                                </button>
                              ))}
                          </div>
                        );
                      })()}
                    </div>
                  </div>
                </div>
              ))}
              {isSending &&
                // Suppress the legacy 3-dot placeholder once streaming
                // output (visible text or thinking) has started — the
                // streaming preview bubble below takes over as the
                // activity indicator.
                !(
                  (selectedStreamingAssistant?.content.length ?? 0) > 0 ||
                  (selectedStreamingAssistant?.thinking.length ?? 0) > 0
                ) && (
                  <div className="flex justify-start">
                    <div className="bg-stone-200/80 rounded-2xl rounded-bl-md px-4 py-3">
                      <div className="flex items-center gap-1">
                        <span className="w-1.5 h-1.5 rounded-full bg-stone-500 animate-bounce [animation-delay:0ms]" />
                        <span className="w-1.5 h-1.5 rounded-full bg-stone-500 animate-bounce [animation-delay:150ms]" />
                        <span className="w-1.5 h-1.5 rounded-full bg-stone-500 animate-bounce [animation-delay:300ms]" />
                      </div>
                    </div>
                  </div>
                )}
              {/* Streaming assistant preview — compact trailing tail of the
                  in-flight response. Rendered as plain text (not Markdown) to
                  avoid jitter from partially-parsed fences. The final bubble
                  replaces this via addInferenceResponse on chat_done. */}
              {selectedStreamingAssistant &&
                (selectedStreamingAssistant.content.length > 0 ||
                  selectedStreamingAssistant.thinking.length > 0) && (
                  <div className="flex justify-start">
                    <div className="relative w-fit max-w-[75%]">
                      {selectedStreamingAssistant.thinking.length > 0 && (
                        <details className="mb-1.5 bg-stone-100 rounded-lg px-3 py-1.5 text-xs text-stone-600 open:bg-stone-100">
                          <summary className="cursor-pointer select-none flex items-center gap-1.5">
                            <span className="inline-block w-1.5 h-1.5 rounded-full bg-primary-400 animate-pulse" />
                            <span>Thinking…</span>
                          </summary>
                          <pre className="whitespace-pre-wrap break-words mt-1.5 font-sans text-[11px] text-stone-500">
                            {selectedStreamingAssistant.thinking.slice(-STREAMING_PREVIEW_CHARS)}
                          </pre>
                        </details>
                      )}
                      {selectedStreamingAssistant.content.length > 0 && (
                        <div className="rounded-2xl rounded-bl-md px-3 py-1.5 bg-stone-200/80 text-stone-900">
                          <p className="text-xs text-stone-700 font-mono whitespace-pre-wrap break-words leading-snug">
                            {selectedStreamingAssistant.content.length >
                              STREAMING_PREVIEW_CHARS && <span className="text-stone-400">…</span>}
                            {selectedStreamingAssistant.content.slice(-STREAMING_PREVIEW_CHARS)}
                            <span className="inline-block w-1 h-3 ml-0.5 align-middle bg-primary-400 animate-pulse" />
                          </p>
                        </div>
                      )}
                    </div>
                  </div>
                )}
              {/* Inference status indicator */}
              {selectedInferenceStatus && (
                <div className="flex items-center gap-2 px-1 py-1.5 text-xs text-stone-500">
                  <span className="inline-block w-2 h-2 rounded-full bg-primary-400 animate-pulse" />
                  <span>
                    {selectedInferenceStatus.phase === 'thinking' &&
                      (selectedInferenceStatus.iteration > 0
                        ? `Thinking (iteration ${selectedInferenceStatus.iteration})...`
                        : 'Thinking...')}
                    {selectedInferenceStatus.phase === 'tool_use' &&
                      `${
                        formatTimelineEntry(
                          activeToolTimelineEntry ?? {
                            id: 'active-tool',
                            name: selectedInferenceStatus.activeTool ?? 'tool',
                            round: selectedInferenceStatus.iteration,
                            status: 'running',
                          }
                        ).title
                      }...`}
                    {selectedInferenceStatus.phase === 'subagent' &&
                      `${
                        formatTimelineEntry(
                          activeSubagentTimelineEntry ?? {
                            id: 'active-subagent',
                            name: `subagent:${selectedInferenceStatus.activeSubagent ?? ''}`,
                            round: selectedInferenceStatus.iteration,
                            status: 'running',
                          }
                        ).title
                      }...`}
                  </span>
                </div>
              )}
              {/* Tool call timeline */}
              {selectedThreadToolTimeline.length > 0 &&
                !shouldRenderTimelineBeforeLatestAgentMessage && (
                  <ToolTimelineBlock entries={selectedThreadToolTimeline} />
                )}
              {isSending && rustChat && (
                <div className="flex justify-start px-1">
                  <button
                    onClick={() => {
                      if (selectedThreadId) void chatCancel(selectedThreadId);
                    }}
                    className="text-xs text-stone-500 hover:text-stone-700 transition-colors">
                    Cancel
                  </button>
                </div>
              )}
              <div ref={messagesEndRef} />
            </div>
          ) : (
            // [#1123] Commented out — welcome-agent onboarding replaced by Joyride walkthrough
            // ) : welcomeThreadId && selectedThreadId === welcomeThreadId ? (
            //   // Welcome thread, no messages yet — the proactive welcome agent
            //   // is running in the background. Show a friendly loader until
            //   // the first agent message lands (which flips us into the
            //   // `hasVisibleMessages` branch above).
            //   <div className="flex-1 flex flex-col items-center justify-center h-full gap-3">
            //     <div className="flex items-center gap-1">
            //       <span className="w-2 h-2 rounded-full bg-stone-500 animate-bounce [animation-delay:0ms]" />
            //       <span className="w-2 h-2 rounded-full bg-stone-500 animate-bounce [animation-delay:150ms]" />
            //       <span className="w-2 h-2 rounded-full bg-stone-500 animate-bounce [animation-delay:300ms]" />
            //     </div>
            //     <WelcomeThinkingTypewriter />
            //   </div>
            <div className="flex-1 flex items-center justify-center h-full">
              <p className="text-sm text-stone-600">No messages yet</p>
            </div>
          )}
        </div>

        <div className="flex-shrink-0 border-t border-stone-200 px-4 py-3">
          {/* [#1123] welcomeLocked and welcomePending guards removed — Joyride walkthrough replaced welcome-agent */}
          <>
            {isNearLimit &&
              !isAtLimit &&
              isFreeTier &&
              shouldShowBanner('conversations-warning', 24 * 60 * 60 * 1000) && (
                <div className="mb-3">
                  <UpsellBanner
                    variant="warning"
                    title="Approaching usage limit"
                    message={`You've used ${Math.round(Math.max(usagePct10h, usagePct7d) * 100)}% of your inference budget. Upgrade for higher limits.`}
                    ctaLabel="Upgrade"
                    onCtaClick={() => {
                      void openUrl(BILLING_DASHBOARD_URL);
                    }}
                    dismissible
                    onDismiss={() => dismissBanner('conversations-warning')}
                  />
                </div>
              )}
            {teamUsage && (shouldShowBudgetCompletedMessage || isRateLimited) && (
              <div className="mb-3 p-3 rounded-xl bg-coral-50 border border-coral-200 flex items-center justify-between gap-3">
                <div className="flex items-center gap-2 min-w-0">
                  <svg
                    className="w-4 h-4 text-coral-400 flex-shrink-0"
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"
                    />
                  </svg>
                  <p className="text-xs text-coral-600 truncate">
                    {shouldShowBudgetCompletedMessage
                      ? teamUsage.cycleBudgetUsd > 0
                        ? `You've hit your weekly limit.${teamUsage.cycleEndsAt ? ` Resets ${formatResetTime(teamUsage.cycleEndsAt)}.` : ''} Top up to continue.`
                        : 'Your included budget is complete. Add credits or upgrade to continue.'
                      : `10-hour rate limit reached.${teamUsage.fiveHourResetsAt ? ` Resets ${formatResetTime(teamUsage.fiveHourResetsAt)}.` : ''}`}
                  </p>
                </div>
                {shouldShowBudgetCompletedMessage && (
                  <button
                    onClick={() => {
                      void openUrl(BILLING_DASHBOARD_URL);
                    }}
                    className="flex-shrink-0 px-3 py-1.5 rounded-lg bg-coral-500 hover:bg-coral-400 text-white text-xs font-medium transition-colors">
                    Top Up
                  </button>
                )}
              </div>
            )}

            {/* Quota / usage pills — hidden during welcome lockdown so the
                  onboarding chat doesn't surface billing affordances. */}
            <div className="flex items-center justify-end gap-2 mb-2">
              {(isLoadingBudget || teamUsage) && (
                <div className="relative group">
                  {teamUsage ? (
                    <div className="flex items-center gap-2">
                      {!teamUsage.bypassCycleLimit && (
                        <LimitPill
                          label="5h"
                          usedPct={
                            teamUsage.fiveHourCapUsd > 0
                              ? Math.min(1, teamUsage.cycleLimit5hr / teamUsage.fiveHourCapUsd)
                              : 0
                          }
                        />
                      )}
                      <LimitPill
                        label="7d"
                        usedPct={
                          teamUsage.cycleBudgetUsd > 0
                            ? Math.min(
                                1,
                                (teamUsage.cycleBudgetUsd - teamUsage.remainingUsd) /
                                  teamUsage.cycleBudgetUsd
                              )
                            : 0
                        }
                      />
                    </div>
                  ) : (
                    <span className="text-[10px] text-stone-400 animate-pulse">loading…</span>
                  )}
                  {teamUsage && (
                    <div className="absolute bottom-full right-0 mb-2 hidden group-hover:block z-50">
                      <div className="bg-stone-900 text-white text-[10px] rounded-lg px-3 py-2 shadow-lg whitespace-nowrap space-y-1.5">
                        {!teamUsage.bypassCycleLimit && (
                          <div className="flex items-center justify-between gap-4">
                            <span className="text-stone-400">5-hour limit</span>
                            <span>
                              ${(teamUsage.cycleLimit5hr ?? 0).toFixed(2)} / $
                              {(teamUsage.fiveHourCapUsd ?? 0).toFixed(2)}
                              {teamUsage.fiveHourResetsAt && (
                                <span className="text-stone-400 ml-1">
                                  — resets {formatResetTime(teamUsage.fiveHourResetsAt)}
                                </span>
                              )}
                            </span>
                          </div>
                        )}
                        <div className="flex items-center justify-between gap-4">
                          <span className="text-stone-400">Weekly limit</span>
                          <span>
                            ${(teamUsage.remainingUsd ?? 0).toFixed(2)} / $
                            {(teamUsage.cycleBudgetUsd ?? 0).toFixed(2)} left
                            {teamUsage.cycleEndsAt && (
                              <span className="text-stone-400 ml-1">
                                — resets {formatResetTime(teamUsage.cycleEndsAt)}
                              </span>
                            )}
                          </span>
                        </div>
                      </div>
                    </div>
                  )}
                </div>
              )}
            </div>
          </>

          {sendAdvisory && (
            <div className="flex items-center justify-between mb-2">
              <p className="text-xs text-amber-700" data-chat-send-advisory>
                {sendAdvisory}
              </p>
              <button
                onClick={() => setSendAdvisory(null)}
                className="text-xs text-stone-500 hover:text-stone-700 transition-colors ml-2">
                Dismiss
              </button>
            </div>
          )}

          {sendError && (
            <div className="flex items-center justify-between mb-2">
              <p className="text-xs text-coral-500" data-chat-send-error-code={sendError.code}>
                {sendError.message}
              </p>
              <div className="flex items-center gap-2 flex-shrink-0 ml-2">
                {(sendError.code === 'stt_not_ready' ||
                  sendError.code === 'voice_transcription') && (
                  <button
                    onClick={() => {
                      setSendError(null);
                      navigate('/settings/local-model');
                    }}
                    className="text-xs text-primary-500 hover:text-primary-600 font-medium transition-colors">
                    Set up
                  </button>
                )}
                <button
                  onClick={() => setSendError(null)}
                  className="text-xs text-stone-500 hover:text-stone-700 transition-colors">
                  Dismiss
                </button>
              </div>
            </div>
          )}

          {composer === 'mic-cloud' ? (
            <MicCloudComposer
              // Without `!selectedThreadId`, a mic submit before a thread is
              // ready hits `handleSendMessage`'s early return and the
              // transcript is silently dropped — the user spoke into the void.
              disabled={composerInteractionBlocked || !selectedThreadId}
              onSubmit={text => handleSendMessage(text)}
              onError={message => setSendError(chatSendError('voice_transcription', message))}
            />
          ) : inputMode === 'text' ? (
            <div className="flex items-end gap-3">
              <div className="relative flex flex-1 items-center justify-center rounded-xl border border-stone-200 bg-white transition-all focus-within:border-primary-500/50 focus-within:ring-1 focus-within:ring-primary-500/50">
                <div
                  aria-hidden
                  className="pointer-events-none absolute inset-0 overflow-hidden whitespace-pre-wrap break-words px-4 py-2.5 text-sm leading-normal font-sans">
                  <span className="invisible">{inputValue}</span>
                  <span className="text-stone-500/50">{inlineCompletionSuffix}</span>
                </div>
                <textarea
                  ref={textInputRef}
                  value={inputValue}
                  onChange={e => setInputValue(e.target.value)}
                  onKeyDown={handleInputKeyDown}
                  placeholder="Type a message..."
                  rows={1}
                  disabled={composerInteractionBlocked}
                  className="relative z-10 w-full resize-none border-0 bg-transparent pl-4 pr-10 py-2.5 text-sm leading-normal whitespace-pre-wrap break-words font-sans text-stone-900 placeholder:text-stone-400 outline-none focus:outline-none focus-visible:outline-none focus:ring-0 focus-visible:ring-0 max-h-32 disabled:opacity-50 disabled:cursor-not-allowed"
                />
                {/* Voice input mic hidden per #717 (inputMode='voice' path retained). */}
              </div>
              <button
                aria-label="Send message"
                title="Send message"
                onClick={() => {
                  void handleSendMessage();
                }}
                disabled={!inputValue.trim() || composerInteractionBlocked}
                className="w-10 h-10 flex items-center justify-center rounded-full bg-primary-500 hover:bg-primary-600 text-white disabled:opacity-40 disabled:cursor-not-allowed transition-colors flex-shrink-0">
                {isSending ? (
                  <svg className="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
                    <circle
                      className="opacity-25"
                      cx="12"
                      cy="12"
                      r="10"
                      stroke="currentColor"
                      strokeWidth="4"
                    />
                    <path
                      className="opacity-75"
                      fill="currentColor"
                      d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
                    />
                  </svg>
                ) : (
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2.5}
                      d="M9 5l7 7-7 7"
                    />
                  </svg>
                )}
              </button>
            </div>
          ) : (
            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={() => setInputMode('text')}
                disabled={isRecording || isTranscribing}
                className="w-10 h-10 flex items-center justify-center rounded-full border border-stone-200 bg-white text-stone-500 hover:text-stone-700 hover:border-stone-300 transition-colors disabled:opacity-40"
                title="Switch to text input">
                <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={1.8}
                    d="M4 6h16M4 12h10m-10 6h16"
                  />
                </svg>
              </button>
              <button
                type="button"
                onClick={() => {
                  void handleVoiceRecordToggle();
                }}
                disabled={!rustChat || isSending || isTranscribing || !canUseMicrophoneApi}
                className={`px-4 py-2.5 rounded-xl text-sm font-medium transition-colors ${
                  isRecording
                    ? 'bg-coral-500 hover:bg-coral-400 text-white'
                    : 'bg-primary-600 hover:bg-primary-500 text-white'
                } disabled:opacity-40 disabled:cursor-not-allowed`}>
                {isTranscribing ? 'Transcribing…' : isRecording ? 'Stop & Send' : 'Start Talking'}
              </button>
              <p className="text-xs text-stone-400 truncate">
                {voiceStatus ??
                  (isPlayingReply && replyMode === 'voice'
                    ? 'Playing voice reply…'
                    : canUseMicrophoneApi
                      ? 'Click "Start Talking" to speak to the agent.'
                      : 'Microphone input is not available in this runtime.')}
              </p>
            </div>
          )}
        </div>
      </div>
      <UsageLimitModal
        open={showLimitModal}
        onClose={() => setShowLimitModal(false)}
        isBudgetExhausted={isBudgetExhausted}
        resetTime={isBudgetExhausted ? teamUsage?.cycleEndsAt : teamUsage?.fiveHourResetsAt}
        currentTier={currentTier}
      />
      <ConfirmationModal
        modal={deleteModal}
        onClose={() => setDeleteModal(prev => ({ ...prev, isOpen: false }))}
      />
    </div>
  );
};

export default Conversations;

/**
 * Embeddable variant — same component, page layout (floating centered
 * card). Mounted inside /accounts when the Agent entry is selected.
 */
export const AgentChatPanel = () => <Conversations variant="page" />;
