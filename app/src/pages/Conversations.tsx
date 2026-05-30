import { convertFileSrc } from '@tauri-apps/api/core';
import debugFactory from 'debug';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import { type ChatSendError, chatSendError } from '../chat/chatSendError';
import { checkPromptInjection, promptGuardMessage } from '../chat/promptInjectionGuard';
import ApprovalRequestCard from '../components/chat/ApprovalRequestCard';
import AttachmentPreview from '../components/chat/AttachmentPreview';
import TokenUsagePill from '../components/chat/TokenUsagePill';
import { ConfirmationModal } from '../components/intelligence/ConfirmationModal';
import PillTabBar from '../components/PillTabBar';
import UpsellBanner from '../components/upsell/UpsellBanner';
import { dismissBanner, shouldShowBanner } from '../components/upsell/upsellDismissState';
import MicComposer from '../features/human/MicComposer';
import { useStickToBottom } from '../hooks/useStickToBottom';
import { useUsageState } from '../hooks/useUsageState';
import {
  ALLOWED_IMAGE_MIME_TYPES,
  type Attachment,
  ATTACHMENT_MAX_IMAGES,
  ATTACHMENT_MAX_SIZE_BYTES,
  buildMessageWithAttachments,
  parseMessageImages,
  validateAndReadFile,
} from '../lib/attachments';
import { useT } from '../lib/i18n/I18nContext';
import { trackEvent } from '../services/analytics';
import { threadApi } from '../services/api/threadApi';
import { chatCancel, chatSend, useRustChat } from '../services/chatService';
import { store } from '../store';
import {
  loadAgentProfiles,
  selectActiveAgentProfileId,
  selectAgentProfile,
  selectAgentProfiles,
  upsertAgentProfile,
} from '../store/agentProfileSlice';
import {
  beginInferenceTurn,
  clearRuntimeForThread,
  fetchAndHydrateTurnState,
  type InferenceStatus,
  setTaskBoardForThread,
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
  updateThreadTitle,
} from '../store/threadSlice';
import type { AgentProfile } from '../types/agentProfile';
import type { ConfirmationModal as ConfirmationModalType } from '../types/intelligence';
import type { ThreadMessage } from '../types/thread';
import type { TaskBoardCard, TaskBoardCardStatus } from '../types/turnState';
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
import { TaskKanbanBoard } from './conversations/components/TaskKanbanBoard';
import { ToolTimelineBlock } from './conversations/components/ToolTimelineBlock';
import {
  evaluateComposerSend,
  getComposerBlockedSendFeedback,
  handleComposerSlashCommand,
} from './conversations/composerSendDecision';
import { runDecidePlan } from './conversations/taskPlanActions';
import {
  type AgentBubblePosition,
  buildAcceptedInlineCompletion,
  formatRelativeTime,
  formatResetTime,
  getInlineCompletionSuffix,
} from './conversations/utils/format';
import { isThreadVisibleInTab, WORKERS_TAB_VALUE } from './conversations/utils/threadFilter';

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
const debug = debugFactory('conversations');
const DEFAULT_PROFILE_DRAFT = {
  name: '',
  agentId: 'orchestrator',
  systemPromptSuffix: '',
  allowedTools: '',
};

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
  rustChat: boolean;
}): boolean {
  return !args.rustChat || Boolean(args.activeThreadId);
}

interface ImeKeyboardEventLike {
  isComposing?: boolean;
  keyCode?: number;
  which?: number;
  nativeEvent?: { isComposing?: boolean; keyCode?: number; which?: number };
}

export function isImeCompositionKeyEvent(event: ImeKeyboardEventLike): boolean {
  return (
    event.isComposing === true ||
    event.nativeEvent?.isComposing === true ||
    event.nativeEvent?.keyCode === 229 ||
    event.nativeEvent?.which === 229 ||
    event.keyCode === 229 ||
    event.which === 229
  );
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

function formatAgentProfileAgentLabel(agentId: string): string {
  return agentId
    .split(/[_-]+/)
    .filter(Boolean)
    .map(part => part.charAt(0).toUpperCase() + part.slice(1))
    .join(' ');
}

const Conversations = ({
  variant = 'page',
  composer: composerProp = 'text',
}: ConversationsProps = {}) => {
  const [composerOverride, setComposerOverride] = useState<'mic-cloud' | 'text' | null>(null);
  const composer = composerOverride ?? composerProp;
  const { t } = useT();
  const dispatch = useAppDispatch();
  const navigate = useNavigate();
  const { threads, selectedThreadId, messages, isLoadingMessages, messagesError, activeThreadId } =
    useAppSelector(state => state.thread);

  const [showSidebar, setShowSidebar] = useState(false);
  const [inputValue, setInputValue] = useState('');
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const fileInputRef = useRef<HTMLInputElement>(null);
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
  const [attachError, setAttachError] = useState<ChatSendError | null>(null);
  const [sendAdvisory, setSendAdvisory] = useState<string | null>(null);
  const [pendingSendingThreadId, setPendingSendingThreadId] = useState<string | null>(null);
  const [profileDraftOpen, setProfileDraftOpen] = useState(false);
  const [profileDraft, setProfileDraft] = useState(DEFAULT_PROFILE_DRAFT);
  const socketStatus = useAppSelector(selectSocketStatus);
  const agentProfiles = useAppSelector(selectAgentProfiles);
  const selectedAgentProfileId = useAppSelector(selectActiveAgentProfileId);
  // Optional chain because narrow test stores (e.g. Conversations.test
  // bootstraps without the locale slice) shouldn't crash here. `'en'`
  // matches the no-locale-directive branch in the core, so legacy
  // behaviour stays intact.
  const uiLocale = useAppSelector(state => state.locale?.current ?? 'en');
  const toolTimelineByThread = useAppSelector(state => state.chatRuntime.toolTimelineByThread);
  const taskBoardByThread = useAppSelector(state => state.chatRuntime.taskBoardByThread);
  const inferenceStatusByThread = useAppSelector(
    state => state.chatRuntime.inferenceStatusByThread
  );
  const pendingApprovalByThread = useAppSelector(
    state => state.chatRuntime.pendingApprovalByThread
  );
  const streamingAssistantByThread = useAppSelector(
    state => state.chatRuntime.streamingAssistantByThread
  );
  const inferenceTurnLifecycleByThread = useAppSelector(
    state => state.chatRuntime.inferenceTurnLifecycleByThread
  );
  const rustChat = useRustChat();
  const [reactionPickerMsgId, setReactionPickerMsgId] = useState<string | null>(null);
  const [editingTitle, setEditingTitle] = useState(false);
  const [editTitleValue, setEditTitleValue] = useState('');
  const editTitleInputRef = useRef<HTMLInputElement>(null);

  const {
    teamUsage,
    isLoading: isLoadingBudget,
    isAtLimit,
    isNearLimit,
    isFreeTier,
    shouldShowBudgetCompletedMessage,
    usagePct,
  } = useUsageState();
  const [deleteModal, setDeleteModal] = useState<ConfirmationModalType>({
    isOpen: false,
    title: '',
    message: '',
    onConfirm: () => {},
    onCancel: () => {},
  });
  const agentProfileAgentOptions = useMemo(() => {
    const seen = new Set<string>();
    const options: Array<{ id: string; label: string }> = [];
    for (const profile of agentProfiles) {
      const id = profile.agentId.trim();
      if (!id || seen.has(id)) continue;
      seen.add(id);
      options.push({
        id,
        label: profile.builtIn ? profile.name : formatAgentProfileAgentLabel(id),
      });
    }
    if (profileDraft.agentId && !seen.has(profileDraft.agentId)) {
      options.push({
        id: profileDraft.agentId,
        label: formatAgentProfileAgentLabel(profileDraft.agentId),
      });
    }
    if (options.length === 0) {
      options.push({ id: 'orchestrator', label: t('chat.agentProfile.defaultAgentLabel') });
    }
    return options;
  }, [agentProfiles, profileDraft.agentId, t]);

  const textInputRef = useRef<HTMLTextAreaElement>(null);
  const isComposingTextRef = useRef(false);
  const pendingSendRef = useRef<string | null>(null);
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
  // Previous inference status for the sending thread; lets the rearm effect
  // distinguish "status was just cleared (chat_done / chat_error)" from
  // "status was never set yet (in-flight turn pre-status)".
  const prevInferenceStatusRef = useRef<InferenceStatus | undefined>(undefined);

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

  const handleStartEditTitle = () => {
    if (!selectedThreadId) return;
    const thr = threads.find(t => t.id === selectedThreadId);
    setEditTitleValue(thr?.title ?? '');
    setEditingTitle(true);
    window.requestAnimationFrame(() => {
      editTitleInputRef.current?.select();
    });
  };

  const handleCommitTitle = () => {
    const trimmed = editTitleValue.trim();
    setEditingTitle(false);
    if (!selectedThreadId || !trimmed) return;
    void dispatch(updateThreadTitle({ threadId: selectedThreadId, title: trimmed }));
  };

  const handleSelectAgentProfile = async (profileId: string) => {
    try {
      await dispatch(selectAgentProfile(profileId)).unwrap();
    } catch (error) {
      debug('agent profile select failed: %o', error);
    }
  };

  const handleCreateAgentProfile = async () => {
    const name = profileDraft.name.trim();
    if (!name) return;
    const duplicate = agentProfiles.some(
      profile => profile.name.trim().toLowerCase() === name.toLowerCase()
    );
    if (duplicate) {
      setSendAdvisory(t('chat.agentProfile.exists').replace('{name}', name));
      return;
    }
    const id = `profile-${globalThis.crypto.randomUUID().slice(0, 8)}`;
    const allowedTools = profileDraft.allowedTools
      .split(',')
      .map(tool => tool.trim())
      .filter(Boolean);
    const profile: AgentProfile = {
      id,
      name,
      description: t('chat.agentProfile.customDescription'),
      agentId: profileDraft.agentId,
      systemPromptSuffix: profileDraft.systemPromptSuffix.trim() || null,
      allowedTools: allowedTools.length > 0 ? allowedTools : null,
      builtIn: false,
    };
    try {
      await dispatch(upsertAgentProfile(profile)).unwrap();
      await dispatch(selectAgentProfile(id)).unwrap();
      setProfileDraftOpen(false);
      setProfileDraft(DEFAULT_PROFILE_DRAFT);
      setSendAdvisory(null);
    } catch (error) {
      debug('agent profile create failed: %o', error);
      setSendAdvisory(t('chat.agentProfile.createFailed'));
    }
  };

  useEffect(() => {
    let cancelled = false;

    void dispatch(loadThreads())
      .unwrap()
      .then(data => {
        if (cancelled) return;
        const threadStateForSelect = store.getState().thread;
        // Worker/subagent threads are hidden from the conversation list
        // (see tinyhumansai/openhuman#1624). Match the sidebar filter here so
        // initial/resume selection can't auto-pick a hidden thread and leave
        // the UI showing a thread that isn't in the list.
        const visibleThreads = data.threads.filter(t => !t.parentThreadId);
        if (visibleThreads.length > 0) {
          // Prefer the thread the user was last viewing (persisted across
          // reloads via redux-persist on the `thread` slice). Only fall
          // through to "most recent" if that thread no longer exists
          // server-side (deleted, purged, or different user) — or is now
          // hidden because it's a worker thread.
          const persistedId = threadStateForSelect.selectedThreadId;
          const resumeId =
            persistedId && visibleThreads.some(t => t.id === persistedId)
              ? persistedId
              : visibleThreads[0].id;
          dispatch(setSelectedThread(resumeId));
          void dispatch(loadThreadMessages(resumeId));
        } else {
          void handleCreateNewThread();
        }
      })
      .catch(err => {
        if (cancelled) return;
        debug('loadThreads failed on mount: %s', formatThreadLoadError(err));
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
      void threadApi
        .getTaskBoard(selectedThreadId)
        .then(board => {
          if (board) {
            dispatch(setTaskBoardForThread({ threadId: selectedThreadId, board }));
          }
        })
        .catch(error => {
          debug('getTaskBoard failed: %o', error);
        });
    }
  }, [selectedThreadId, dispatch]);

  useEffect(() => {
    void dispatch(loadAgentProfiles())
      .unwrap()
      .catch(error => {
        debug('agent profiles load failed: %o', error);
      });
  }, [dispatch]);

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
      debug('armSilenceTimer: no inference signal for 120s — clearing runtime');
      setSendError(chatSendError('safety_timeout', t('chat.safetyTimeout')));
      dispatch(clearRuntimeForThread({ threadId }));
      dispatch(setActiveThread(null));
      sendingTimeoutRef.current = null;
      sendingThreadIdRef.current = null;
      // Reset so the NEXT send starts from a clean "never had a status"
      // baseline — otherwise the rearm effect could read this turn's last
      // status as a stale "previous" and falsely treat the next send's
      // first signal as a chat-done transition.
      prevInferenceStatusRef.current = undefined;
      pendingSendRef.current = null;
      setPendingSendingThreadId(null);
    }, 120_000);
  };

  // Rearm the silence timer on every inference signal for the sending
  // thread. Top-level tool / iteration events bump `inferenceStatusByThread`;
  // pure-text streams (no tools) only bump `streamingAssistantByThread`;
  // sub-agent activity (a delegated `Research`/`Tools Agent`/`Memory Tree`
  // turn whose tools run in a child task) bumps `toolTimelineByThread` and
  // `taskBoardByThread` without necessarily re-emitting a top-level status
  // change, so all four must be watched — otherwise a long sub-agent loop
  // would trip the safety timer mid-run even though the user can see the
  // delegated tools firing in the timeline. When the status is cleared
  // (chat_done / chat_error), drop the timer — the completion handlers
  // own UI cleanup.
  //
  // `prevInferenceStatusRef` distinguishes "status was just cleared
  // (chat_done / chat_error transition: defined → undefined)" from "status
  // was never set yet (the Send handler also dispatches
  // `setToolTimelineForThread({ entries: [] })` to reset the timeline,
  // which fires this effect immediately after `armSilenceTimer` — at
  // that instant the inference status hasn't been published yet)". Only
  // the real transition should clear our timer.
  useEffect(() => {
    const threadId = sendingThreadIdRef.current;
    if (!threadId || !sendingTimeoutRef.current) return;
    const status = inferenceStatusByThread[threadId];
    if (status === undefined && prevInferenceStatusRef.current !== undefined) {
      clearTimeout(sendingTimeoutRef.current);
      sendingTimeoutRef.current = null;
      sendingThreadIdRef.current = null;
      prevInferenceStatusRef.current = undefined;
      return;
    }
    prevInferenceStatusRef.current = status;
    armSilenceTimer(threadId);
    // Scope the dependencies to the SENDING thread's slices only, keyed by the
    // reactive `activeThreadId` (set on send, cleared on done/error/timeout —
    // so it tracks the in-flight turn for the timer's whole lifetime, unlike
    // `pendingSendingThreadId` which is released the moment the backend accepts
    // the send). Depending on the whole maps would rearm this thread's timer
    // whenever ANY other thread's state changed — unrelated background activity
    // shouldn't keep a foreground turn's timer alive. armSilenceTimer is stable
    // (refs + dispatch), so listing the per-thread values is enough to rearm on
    // every progress event for this thread.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    activeThreadId,
    activeThreadId ? inferenceStatusByThread[activeThreadId] : undefined,
    activeThreadId ? streamingAssistantByThread[activeThreadId] : undefined,
    activeThreadId ? toolTimelineByThread[activeThreadId] : undefined,
    activeThreadId ? taskBoardByThread[activeThreadId] : undefined,
  ]);

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
    const decision = handleComposerSlashCommand(command);
    if (decision.kind === 'not_handled') return false;

    setInputValue('');
    void handleCreateNewThread();
    return true;
  };

  const handleAttachFiles = async (files: FileList | null) => {
    if (!files) return;
    let acceptedCount = attachments.length;
    for (const file of Array.from(files)) {
      const result = await validateAndReadFile(file, acceptedCount);
      if ('error' in result) {
        const { error } = result;
        if (error.code === 'too_many') {
          setAttachError(
            chatSendError(
              'attachment_invalid',
              t('chat.attachment.tooMany').replace('{max}', String(ATTACHMENT_MAX_IMAGES))
            )
          );
        } else if (error.code === 'too_large') {
          const maxMb = (ATTACHMENT_MAX_SIZE_BYTES / (1024 * 1024)).toFixed(0);
          setAttachError(
            chatSendError(
              'attachment_invalid',
              t('chat.attachment.tooLarge').replace('{max}', `${maxMb} MB`)
            )
          );
        } else if (error.code === 'unsupported_type') {
          setAttachError(chatSendError('attachment_invalid', t('chat.attachment.unsupportedType')));
        } else {
          setAttachError(chatSendError('attachment_invalid', t('chat.attachment.readFailed')));
        }
        return;
      }
      acceptedCount++;
      setAttachments(prev => [...prev, result.attachment]);
    }
  };

  const handleSendMessage = async (text?: string) => {
    if (pendingSendRef.current) return;

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
      (sendDecision.blockReason === 'empty_input' && attachments.length === 0) ||
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

    if (
      !sendDecision.shouldSend &&
      !(sendDecision.blockReason === 'empty_input' && attachments.length > 0)
    ) {
      const blockedFeedback = getComposerBlockedSendFeedback(sendDecision.blockReason);
      if (blockedFeedback) {
        setSendError(chatSendError(blockedFeedback.error.code, blockedFeedback.error.message));
      }
      return;
    }

    const sendingThreadId = selectedThreadId;
    if (!sendingThreadId) return;
    pendingSendRef.current = sendingThreadId;
    setPendingSendingThreadId(sendingThreadId);
    const pendingAttachments = attachments.slice();
    const messageText = buildMessageWithAttachments(trimmed, pendingAttachments);
    const userMessage: ThreadMessage = {
      id: `msg_${globalThis.crypto.randomUUID()}`,
      content: trimmed,
      type: 'text',
      extraMetadata:
        pendingAttachments.length > 0
          ? {
              attachmentCount: pendingAttachments.length,
              attachmentNames: pendingAttachments.map(a => a.file.name),
              attachmentDataUris: pendingAttachments.map(a => a.dataUri),
            }
          : {},
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
        pendingSendRef.current = null;
        setPendingSendingThreadId(null);
        return;
      }
      const msg = error instanceof Error ? error.message : String(error);
      setSendError(chatSendError('cloud_send_failed', msg));
      pendingSendRef.current = null;
      setPendingSendingThreadId(null);
      return;
    }
    setInputValue('');
    setAttachments([]);
    setSendError(null);
    setAttachError(null);
    // Silence timer: fires only if 600s pass without ANY inference progress
    // (tool call, tool result, iteration start, subagent event, text delta).
    // The effect below rearms this timer whenever `inferenceStatusByThread`
    // changes for `sendingThreadId`, so long-running agent turns stay alive
    // as long as the backend is emitting signals. A truly hung server still
    // fails fast.
    // Fresh send: clear the previous-status baseline before arming so the
    // first inference signal of this turn isn't misread as a chat-done
    // transition (defined → undefined) left over from the prior turn.
    prevInferenceStatusRef.current = undefined;
    armSilenceTimer(sendingThreadId);
    dispatch(setToolTimelineForThread({ threadId: sendingThreadId, entries: [] }));
    dispatch(beginInferenceTurn({ threadId: sendingThreadId }));
    dispatch(setActiveThread(sendingThreadId));

    // ── Cloud socket path ─────────────────────────────────────────────────────
    // Always route primary chat through the cloud backend via socket.
    // Local model (Ollama) is used only for supplementary features
    // (auto-react, autocomplete, etc.) — never as a primary chat path.
    try {
      await chatSend({
        threadId: sendingThreadId,
        message: messageText,
        model: CHAT_MODEL_ID,
        profileId: selectedAgentProfileId,
        locale: uiLocale,
      });
      trackEvent('chat_message_sent');
      // Backend accepted the send; lifecycle ('started' → 'streaming') now
      // owns the `isSending` UI lock. Release the pending guard so the next
      // user turn isn't blocked by a stale ref/state.
      pendingSendRef.current = null;
      setPendingSendingThreadId(null);

      // Active-thread reset happens in the global ChatRuntimeProvider events.
    } catch (err) {
      // Chat loop errors are emitted via socket events; this catch handles emit-level failures.
      if (sendingTimeoutRef.current) {
        clearTimeout(sendingTimeoutRef.current);
        sendingTimeoutRef.current = null;
      }
      sendingThreadIdRef.current = null;
      prevInferenceStatusRef.current = undefined;
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
      pendingSendRef.current = null;
      setPendingSendingThreadId(null);
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
    if (isComposingTextRef.current || isImeCompositionKeyEvent(e)) return;

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
  const selectedTaskBoard = selectedThreadId ? (taskBoardByThread[selectedThreadId] ?? null) : null;
  const hasTaskBoard = Boolean(selectedTaskBoard?.cards.length);
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
  // Blocks all composer interaction while a turn is in-flight or Rust chat is unavailable.
  // isSending: the *selected* thread is in-flight (drives selected-thread UI only).
  const composerInteractionBlocked = isComposerInteractionBlocked({ activeThreadId, rustChat });
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
    (pendingSendingThreadId === selectedThreadId ||
      inferenceTurnLifecycleByThread[selectedThreadId] === 'started' ||
      inferenceTurnLifecycleByThread[selectedThreadId] === 'streaming')
  );
  const shouldRenderTimelineBeforeLatestAgentMessage =
    selectedThreadToolTimeline.length > 0 && !isSending && Boolean(latestVisibleAgentMessage);

  const handleMoveTaskCard = async (
    card: TaskBoardCard,
    nextStatus: TaskBoardCardStatus
  ): Promise<void> => {
    if (!selectedThreadId || !selectedTaskBoard) return;
    const now = new Date().toISOString();
    const nextBoard = {
      ...selectedTaskBoard,
      cards: selectedTaskBoard.cards.map(existing =>
        existing.id === card.id ? { ...existing, status: nextStatus, updatedAt: now } : existing
      ),
      updatedAt: now,
    };
    dispatch(setTaskBoardForThread({ threadId: selectedThreadId, board: nextBoard }));
    try {
      const saved = await threadApi.putTaskBoard(selectedThreadId, nextBoard.cards);
      if (!saved) {
        throw new Error('Task board update returned no board');
      }
      dispatch(setTaskBoardForThread({ threadId: selectedThreadId, board: saved }));
    } catch (error) {
      debug('putTaskBoard failed: %o', error);
      setSendAdvisory(t('conversations.taskKanban.updateFailed'));
      dispatch(setTaskBoardForThread({ threadId: selectedThreadId, board: selectedTaskBoard }));
    }
  };

  const handleUpdateTaskCard = async (
    card: TaskBoardCard,
    nextCard: TaskBoardCard
  ): Promise<void> => {
    if (!selectedThreadId || !selectedTaskBoard) return;
    const now = new Date().toISOString();
    const nextBoard = {
      ...selectedTaskBoard,
      cards: selectedTaskBoard.cards.map(existing =>
        existing.id === card.id ? { ...nextCard, updatedAt: now } : existing
      ),
      updatedAt: now,
    };
    dispatch(setTaskBoardForThread({ threadId: selectedThreadId, board: nextBoard }));
    try {
      const saved = await threadApi.putTaskBoard(selectedThreadId, nextBoard.cards);
      if (!saved) {
        throw new Error('Task board update returned no board');
      }
      dispatch(setTaskBoardForThread({ threadId: selectedThreadId, board: saved }));
    } catch (error) {
      debug('putTaskBoard failed: %o', error);
      setSendAdvisory(t('conversations.taskKanban.updateFailed'));
      dispatch(setTaskBoardForThread({ threadId: selectedThreadId, board: selectedTaskBoard }));
    }
  };

  const filteredThreads = useMemo(() => {
    // Worker/subagent threads (any thread with `parentThreadId`) are
    // surfaced through two intentional paths (issue #1624):
    //   1. The dedicated `Workers` tab in the sidebar — pick that tab to
    //      see only background work and jump into a worker transcript.
    //   2. Inline inside the parent thread via `WorkerThreadRefCard`,
    //      which now also renders a live running/completed/failed badge
    //      derived from the parent timeline entry's status.
    // The default ("All") and label-scoped tabs hide them so the main
    // sidebar is dominated by user-initiated conversations rather than
    // background reasoning threads. The actual rule lives in
    // `isThreadVisibleInTab` so it is pure, unit-testable, and stays
    // in lockstep with the sidebar tab definition (`labelTabs` below)
    // via the shared `WORKERS_TAB_VALUE` sentinel.
    return threads.filter(t => isThreadVisibleInTab(t, selectedLabel));
  }, [threads, selectedLabel]);

  const sortedThreads = useMemo(() => {
    return [...filteredThreads].sort(
      (a, b) => new Date(b.lastMessageAt).getTime() - new Date(a.lastMessageAt).getTime()
    );
  }, [filteredThreads]);

  // Fixed tab set so categories don't disappear when empty and the active
  // filter state remains unambiguous regardless of what threads exist.
  // The `workers` tab (issue #1624) is the deliberate UI surface for
  // background sub-agent / worker threads — selecting it inverts the
  // default `parentThreadId` filter in `filteredThreads` above so only
  // worker threads show. Without this tab the only way into a worker
  // transcript is the inline `WorkerThreadRefCard` inside the parent.
  const labelTabs = [
    { label: t('chat.filter.all'), value: 'all' },
    { label: t('chat.filter.work'), value: 'work' },
    { label: t('chat.filter.briefing'), value: 'briefing' },
    { label: t('chat.filter.notification'), value: 'notification' },
    { label: t('chat.filter.workers'), value: WORKERS_TAB_VALUE },
  ];

  const isSidebar = variant === 'sidebar';
  const effectiveShowSidebar = showSidebar;

  // Stable title resolver used by both the sidebar thread list and the header.
  const resolveThreadDisplayTitle = (threadId: string | null): string => {
    if (!threadId) return t('chat.selectThread');
    const thr = threads.find(th => th.id === threadId);
    return thr?.title ?? t('chat.selectThread');
  };

  // Resolve the parent of the currently-selected thread, if any. Used to
  // render the back-to-parent breadcrumb in the chat header so a user who
  // dropped into a worker thread (via `WorkerThreadRefCard` or the
  // `Workers` sidebar tab) can return to the conversation that spawned it
  // — issue #1624 acceptance criterion "Parent ↔ worker navigation is
  // bidirectional". Returns `null` when the active thread is a top-level
  // conversation (no parent), so the header stays unchanged in the
  // non-worker case.
  const selectedThreadParent = useMemo(() => {
    if (!selectedThreadId) return null;
    const current = threads.find(thr => thr.id === selectedThreadId);
    const parentId = current?.parentThreadId;
    if (!parentId) return null;
    const parent = threads.find(thr => thr.id === parentId);
    return parent
      ? { id: parent.id, title: parent.title || t('chat.parentThread') }
      : { id: parentId, title: t('chat.parentThread') };
  }, [threads, selectedThreadId, t]);

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
        <div className="w-64 flex-shrink-0 flex flex-col bg-white dark:bg-neutral-900 rounded-2xl shadow-soft border border-stone-200 dark:border-neutral-800 overflow-hidden">
          <div className="flex items-center justify-between px-4 py-3 border-b border-stone-100 dark:border-neutral-800">
            <h2 className="text-sm font-semibold text-stone-700 dark:text-neutral-200">
              {t('chat.threads')}
            </h2>
            <button
              data-testid="new-thread-sidebar-button"
              onClick={() => void handleCreateNewThread()}
              className="w-7 h-7 flex items-center justify-center rounded-lg hover:bg-stone-100 dark:hover:bg-neutral-800 dark:bg-neutral-800 dark:hover:bg-neutral-800/60 text-stone-500 dark:text-neutral-400 hover:text-stone-700 dark:hover:text-neutral-200 dark:text-neutral-200 dark:hover:text-neutral-200 transition-colors"
              title={t('chat.newThread')}>
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
          <div className="px-4 py-2 border-b border-stone-50 dark:border-neutral-800">
            <PillTabBar
              items={labelTabs}
              selected={selectedLabel}
              onChange={setSelectedLabel}
              containerClassName="flex gap-1 overflow-x-auto py-1 scrollbar-hide"
            />
          </div>
          <div className="flex-1 overflow-y-auto">
            {sortedThreads.length === 0 ? (
              <p className="px-4 py-6 text-xs text-stone-400 dark:text-neutral-500 text-center">
                {selectedLabel === 'all'
                  ? t('chat.noThreads')
                  : selectedLabel === WORKERS_TAB_VALUE
                    ? t('chat.noWorkerThreads')
                    : t('chat.noLabelThreads').replace('{label}', selectedLabel)}
              </p>
            ) : (
              sortedThreads.map(thread => (
                <div
                  key={thread.id}
                  data-testid={`thread-row-${thread.id}`}
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
                  className={`w-full text-left px-4 py-3 border-b border-stone-50 dark:border-neutral-800 transition-colors group cursor-pointer ${
                    selectedThreadId === thread.id
                      ? 'bg-primary-50 dark:bg-primary-900/30 border-l-2 border-l-primary-500'
                      : 'hover:bg-stone-50 dark:hover:bg-neutral-800/60'
                  }`}>
                  <div className="flex items-center justify-between">
                    <p
                      className={`text-sm truncate flex-1 ${
                        selectedThreadId === thread.id
                          ? 'font-medium text-primary-700 dark:text-primary-200'
                          : 'text-stone-700 dark:text-neutral-200'
                      }`}>
                      {resolveThreadDisplayTitle(thread.id)}
                    </p>
                    <button
                      onClick={e => {
                        e.stopPropagation();
                        setDeleteModal({
                          isOpen: true,
                          title: t('chat.deleteThread'),
                          message: t('chat.deleteThreadConfirm').replace(
                            '{title}',
                            thread.title || t('chat.untitledThread')
                          ),
                          confirmText: t('common.delete'),
                          cancelText: t('common.cancel'),
                          destructive: true,
                          onConfirm: () => {
                            void dispatch(deleteThread(thread.id));
                          },
                          onCancel: () => {},
                        });
                      }}
                      className="ml-2 p-1 rounded opacity-0 group-hover:opacity-100 hover:bg-stone-200 dark:bg-neutral-800 dark:hover:bg-neutral-800 text-stone-400 dark:text-neutral-500 hover:text-coral-500 transition-all flex-shrink-0"
                      title={t('chat.deleteThread')}>
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
                    <span className="text-[10px] text-stone-400 dark:text-neutral-500">
                      {formatRelativeTime(thread.lastMessageAt)}
                    </span>
                    {thread.messageCount > 0 && (
                      <span className="text-[10px] text-stone-400 dark:text-neutral-500">
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
            ? 'flex-1 flex flex-col min-w-0 bg-white dark:bg-neutral-900 border-l border-stone-200 dark:border-neutral-800 overflow-hidden'
            : 'flex-1 flex flex-col min-w-0 max-w-2xl bg-white dark:bg-neutral-900 rounded-2xl shadow-soft border border-stone-200 dark:border-neutral-800 overflow-hidden'
        }>
        {/* Chat header — only shown in page mode; the sidebar embed uses the
            parent page's chrome instead. Hidden entirely during welcome
            lockdown (#883) so the onboarding chat is just the conversation
            with no chrome around it. */}
        {!isSidebar && (
          <div
            className="flex items-center gap-2 px-4 py-2.5 border-b border-stone-100 dark:border-neutral-800"
            data-walkthrough="chat-agent-panel">
            <button
              onClick={() => setShowSidebar(prev => !prev)}
              className="w-7 h-7 flex items-center justify-center rounded-lg hover:bg-stone-100 dark:hover:bg-neutral-800 dark:bg-neutral-800 dark:hover:bg-neutral-800/60 text-stone-500 dark:text-neutral-400 hover:text-stone-700 dark:hover:text-neutral-200 dark:text-neutral-200 dark:hover:text-neutral-200 transition-colors"
              title={effectiveShowSidebar ? t('chat.hideSidebar') : t('chat.showSidebar')}>
              <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M4 6h16M4 12h16M4 18h16"
                />
              </svg>
            </button>
            <div className="flex flex-col min-w-0 flex-1">
              {selectedThreadParent ? (
                <button
                  type="button"
                  onClick={() => {
                    dispatch(setSelectedThread(selectedThreadParent.id));
                    void dispatch(loadThreadMessages(selectedThreadParent.id));
                  }}
                  className="self-start flex items-center gap-1 text-[11px] font-medium text-primary-600 hover:text-primary-700 hover:underline focus:outline-none focus-visible:ring-2 focus-visible:ring-primary-300 rounded -mx-1 px-1"
                  data-testid="worker-thread-back-to-parent">
                  <span aria-hidden="true">←</span>
                  <span className="truncate max-w-[16rem]">
                    {t('chat.backToThread').replace('{title}', selectedThreadParent.title)}
                  </span>
                </button>
              ) : null}
              {editingTitle ? (
                <input
                  ref={editTitleInputRef}
                  value={editTitleValue}
                  onChange={e => setEditTitleValue(e.target.value)}
                  onKeyDown={e => {
                    if (e.key === 'Enter') {
                      e.preventDefault();
                      handleCommitTitle();
                    } else if (e.key === 'Escape') {
                      setEditingTitle(false);
                    }
                  }}
                  onBlur={handleCommitTitle}
                  aria-label={t('chat.editThreadTitle')}
                  className="h-5 text-sm font-medium text-stone-700 dark:text-neutral-200 bg-transparent border-b border-primary-400 outline-none w-full min-w-0 leading-none py-0"
                  autoFocus
                />
              ) : (
                <div className="flex items-center gap-1 group/title min-w-0">
                  <h3 className="text-sm font-medium text-stone-700 dark:text-neutral-200 truncate">
                    {resolveThreadDisplayTitle(selectedThreadId)}
                  </h3>
                  {selectedThreadId && (
                    <button
                      type="button"
                      onClick={handleStartEditTitle}
                      aria-label={t('chat.editThreadTitle')}
                      title={t('chat.editThreadTitle')}
                      className="opacity-0 group-hover/title:opacity-100 flex-shrink-0 w-5 h-5 flex items-center justify-center rounded hover:bg-stone-100 dark:hover:bg-neutral-800 text-stone-400 dark:text-neutral-500 hover:text-stone-600 dark:hover:text-neutral-300 transition-all">
                      <svg
                        className="w-3 h-3"
                        fill="none"
                        stroke="currentColor"
                        viewBox="0 0 24 24">
                        <path
                          strokeLinecap="round"
                          strokeLinejoin="round"
                          strokeWidth={2}
                          d="M15.232 5.232l3.536 3.536m-2.036-5.036a2.5 2.5 0 113.536 3.536L6.5 21.036H3v-3.572L16.732 3.732z"
                        />
                      </svg>
                    </button>
                  )}
                </div>
              )}
            </div>
            <>
              <div className="flex items-center gap-1">
                <select
                  aria-label={t('chat.agentProfile.label')}
                  value={selectedAgentProfileId}
                  onChange={event => void handleSelectAgentProfile(event.target.value)}
                  className="h-7 max-w-[120px] rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-2 text-xs text-stone-700 dark:text-neutral-200 outline-none transition-colors focus:border-primary-400">
                  {agentProfiles.map(profile => (
                    <option key={profile.id} value={profile.id}>
                      {profile.name}
                    </option>
                  ))}
                </select>
                <button
                  type="button"
                  onClick={() => setProfileDraftOpen(prev => !prev)}
                  className="h-7 w-7 rounded-lg text-xs font-medium text-stone-500 dark:text-neutral-400 transition-colors hover:bg-stone-100 dark:hover:bg-neutral-800 dark:bg-neutral-800 dark:hover:bg-neutral-800/60 hover:text-stone-700 dark:hover:text-neutral-200 dark:text-neutral-200 dark:hover:text-neutral-200"
                  title={t('chat.agentProfile.create')}
                  aria-label={t('chat.agentProfile.create')}>
                  +
                </button>
              </div>
              <TokenUsagePill />
              <button
                data-testid="new-thread-button"
                onClick={() => void handleCreateNewThread()}
                className="px-2.5 py-1 rounded-lg text-xs font-medium text-primary-600 hover:bg-primary-50 transition-colors"
                title={t('chat.newThreadShortcut')}>
                {t('chat.new')}
              </button>
            </>
          </div>
        )}
        {!isSidebar && profileDraftOpen && (
          <div className="border-b border-stone-100 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-4 py-3">
            <div className="grid grid-cols-1 gap-2 sm:grid-cols-[1fr_140px]">
              <input
                value={profileDraft.name}
                onChange={event => setProfileDraft(prev => ({ ...prev, name: event.target.value }))}
                placeholder={t('chat.agentProfile.namePlaceholder')}
                className="h-8 rounded-lg border border-stone-200 dark:border-neutral-800 dark:bg-neutral-900 dark:text-neutral-200 px-3 text-xs outline-none focus:border-primary-400"
              />
              <select
                value={profileDraft.agentId}
                onChange={event =>
                  setProfileDraft(prev => ({ ...prev, agentId: event.target.value }))
                }
                className="h-8 rounded-lg border border-stone-200 dark:border-neutral-800 dark:bg-neutral-900 dark:text-neutral-200 px-2 text-xs outline-none focus:border-primary-400">
                {agentProfileAgentOptions.map(agent => (
                  <option key={agent.id} value={agent.id}>
                    {agent.label}
                  </option>
                ))}
              </select>
            </div>
            <textarea
              value={profileDraft.systemPromptSuffix}
              onChange={event =>
                setProfileDraft(prev => ({ ...prev, systemPromptSuffix: event.target.value }))
              }
              placeholder={t('chat.agentProfile.promptStylePlaceholder')}
              rows={2}
              className="mt-2 w-full resize-none rounded-lg border border-stone-200 dark:border-neutral-800 dark:bg-neutral-900 dark:text-neutral-200 px-3 py-2 text-xs outline-none focus:border-primary-400"
            />
            <div className="mt-2 flex items-center gap-2">
              <input
                value={profileDraft.allowedTools}
                onChange={event =>
                  setProfileDraft(prev => ({ ...prev, allowedTools: event.target.value }))
                }
                placeholder={t('chat.agentProfile.allowedToolsPlaceholder')}
                className="h-8 min-w-0 flex-1 rounded-lg border border-stone-200 dark:border-neutral-800 dark:bg-neutral-900 dark:text-neutral-200 px-3 text-xs outline-none focus:border-primary-400"
              />
              <button
                type="button"
                onClick={() => void handleCreateAgentProfile()}
                disabled={!profileDraft.name.trim()}
                className="h-8 rounded-lg bg-primary-500 px-3 text-xs font-medium text-white transition-colors hover:bg-primary-600 disabled:opacity-40">
                {t('common.save')}
              </button>
              <button
                type="button"
                onClick={() => {
                  setProfileDraft(DEFAULT_PROFILE_DRAFT);
                  setProfileDraftOpen(false);
                }}
                className="h-8 rounded-lg border border-stone-200 dark:border-neutral-800 px-3 text-xs font-medium text-stone-600 dark:text-neutral-300 transition-colors hover:bg-stone-50 dark:hover:bg-neutral-800/60">
                {t('common.cancel')}
              </button>
            </div>
          </div>
        )}
        <div
          ref={messagesContainerRef}
          className="flex-1 overflow-y-auto px-5 py-4 bg-[#f6f6f6] dark:bg-neutral-950">
          {isLoadingMessages ? (
            <div className="space-y-4">
              {Array.from({ length: 4 }).map((_, i) => (
                <div key={i} className={`flex ${i % 2 === 0 ? 'justify-start' : 'justify-end'}`}>
                  <div
                    className={`h-12 rounded-2xl animate-pulse bg-stone-100 dark:bg-neutral-800 ${
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
              <p className="text-sm text-stone-400 dark:text-neutral-500 mb-1">
                {t('chat.failedToLoadMessages')}
              </p>
              <p className="text-xs text-stone-600 dark:text-neutral-300 mb-3 text-center">
                {messagesError}
              </p>
              <button
                onClick={() => window.location.reload()}
                className="text-xs text-primary-400 hover:text-primary-300 transition-colors">
                {t('common.reload')}
              </button>
            </div>
          ) : hasVisibleMessages || hasTaskBoard ? (
            <div className="space-y-3">
              {selectedTaskBoard && hasTaskBoard && (
                <TaskKanbanBoard
                  board={selectedTaskBoard}
                  disabled={!selectedThreadId}
                  onMove={(card, status) => {
                    void handleMoveTaskCard(card, status);
                  }}
                  onUpdateCard={(card, nextCard) => {
                    void handleUpdateTaskCard(card, nextCard);
                  }}
                  onDecidePlan={(card, approve) => {
                    void runDecidePlan({
                      threadId: selectedThreadId,
                      card,
                      approve,
                      dispatch,
                      notify: setSendAdvisory,
                      t,
                    });
                  }}
                />
              )}
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
                            <p className="px-1 text-[10px] text-stone-400 dark:text-neutral-500">
                              {formatRelativeTime(msg.createdAt)}
                            </p>
                          )}
                        </div>
                      ) : (
                        <div className="flex flex-col items-end gap-1">
                          {(() => {
                            const dataUris = Array.isArray(msg.extraMetadata?.attachmentDataUris)
                              ? (msg.extraMetadata.attachmentDataUris as string[])
                              : parseMessageImages(msg.content ?? '').dataUris;
                            const hasImages = dataUris.length > 0;
                            const showTime = latestVisibleMessage?.id === msg.id;
                            return (
                              <>
                                {hasImages && (
                                  <div className="flex flex-wrap gap-1.5 justify-end">
                                    {dataUris.map((uri, i) => (
                                      <img
                                        key={i}
                                        src={uri}
                                        alt=""
                                        className="max-w-[200px] max-h-[200px] rounded-2xl object-cover"
                                      />
                                    ))}
                                  </div>
                                )}
                                {(msg.content || showTime) && (
                                  <div className="rounded-2xl px-4 py-2.5 bg-primary-500 text-white rounded-br-md break-words overflow-hidden">
                                    {msg.content && (
                                      <BubbleMarkdown content={msg.content} tone="user" />
                                    )}
                                    {showTime && (
                                      <p
                                        className={`${msg.content ? 'mt-1' : ''} text-[10px] text-white/60`}>
                                        {formatRelativeTime(msg.createdAt)}
                                      </p>
                                    )}
                                  </div>
                                )}
                              </>
                            );
                          })()}
                        </div>
                      )}
                      <button
                        onClick={() => handleCopyMessage(msg.id, msg.content)}
                        className={`absolute -top-1 ${msg.sender === 'user' ? '-left-8' : '-right-8'} p-1 rounded-md opacity-0 group-hover/msg:opacity-100 hover:bg-stone-100 dark:hover:bg-neutral-800 dark:bg-neutral-800 dark:hover:bg-neutral-800 text-stone-400 dark:text-neutral-500 hover:text-stone-600 dark:hover:text-neutral-300 transition-all`}
                        title={t('chat.copyResponse')}>
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
                                title={t('chat.removeReaction').replace('{emoji}', emoji)}>
                                {emoji}
                              </button>
                            ))}
                            {msg.sender === 'agent' &&
                              (reactionPickerMsgId === msg.id ? (
                                <div className="flex items-center gap-0.5 px-1 py-0.5 rounded-full bg-stone-100 dark:bg-neutral-800">
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
                                    className="ml-0.5 text-stone-600 dark:text-neutral-300 hover:text-stone-400 dark:hover:text-neutral-500 text-xs px-0.5">
                                    ✕
                                  </button>
                                </div>
                              ) : (
                                <button
                                  onClick={() => setReactionPickerMsgId(msg.id)}
                                  className="opacity-0 group-hover/msg:opacity-100 flex items-center px-1.5 py-0.5 rounded-full bg-stone-50 dark:bg-neutral-800/60 hover:bg-stone-200 dark:bg-neutral-800 dark:hover:bg-neutral-800 text-stone-500 dark:text-neutral-400 hover:text-stone-300 dark:hover:text-neutral-600 text-xs transition-all"
                                  title={t('chat.addReaction')}>
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
                    <div className="bg-stone-200/80 dark:bg-neutral-800 rounded-2xl rounded-bl-md px-4 py-3">
                      <div className="flex items-center gap-1">
                        <span className="w-1.5 h-1.5 rounded-full bg-stone-50 dark:bg-neutral-800/600 animate-bounce [animation-delay:0ms]" />
                        <span className="w-1.5 h-1.5 rounded-full bg-stone-50 dark:bg-neutral-800/600 animate-bounce [animation-delay:150ms]" />
                        <span className="w-1.5 h-1.5 rounded-full bg-stone-50 dark:bg-neutral-800/600 animate-bounce [animation-delay:300ms]" />
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
                        <details className="mb-1.5 bg-stone-100 dark:bg-neutral-800 rounded-lg px-3 py-1.5 text-xs text-stone-600 dark:text-neutral-300 open:bg-stone-100 dark:bg-neutral-800 dark:open:bg-neutral-800">
                          <summary className="cursor-pointer select-none flex items-center gap-1.5">
                            <span className="inline-block w-1.5 h-1.5 rounded-full bg-primary-400 animate-pulse" />
                            <span>{t('chat.thinking')}</span>
                          </summary>
                          <pre className="whitespace-pre-wrap break-words mt-1.5 font-sans text-[11px] text-stone-500 dark:text-neutral-400">
                            {selectedStreamingAssistant.thinking.slice(-STREAMING_PREVIEW_CHARS)}
                          </pre>
                        </details>
                      )}
                      {selectedStreamingAssistant.content.length > 0 && (
                        <div className="rounded-2xl rounded-bl-md px-3 py-1.5 bg-stone-200/80 dark:bg-neutral-800 text-stone-900 dark:text-neutral-100">
                          <p className="text-xs text-stone-700 dark:text-neutral-200 font-mono whitespace-pre-wrap break-words leading-snug">
                            {selectedStreamingAssistant.content.length >
                              STREAMING_PREVIEW_CHARS && (
                              <span className="text-stone-400 dark:text-neutral-500">…</span>
                            )}
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
                <div className="flex items-center gap-2 px-1 py-1.5 text-xs text-stone-500 dark:text-neutral-400">
                  <span className="inline-block w-2 h-2 rounded-full bg-primary-400 animate-pulse" />
                  <span>
                    {selectedInferenceStatus.phase === 'thinking' &&
                      (selectedInferenceStatus.iteration > 0
                        ? t('chat.thinkingIteration').replace(
                            '{n}',
                            String(selectedInferenceStatus.iteration)
                          )
                        : t('chat.thinkingDots'))}
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
                    className="text-xs text-stone-500 dark:text-neutral-400 hover:text-stone-700 dark:hover:text-neutral-200 dark:text-neutral-200 dark:hover:text-neutral-200 transition-colors">
                    {t('common.cancel')}
                  </button>
                </div>
              )}
              <div ref={messagesEndRef} />
            </div>
          ) : (
            <div className="flex-1 flex items-center justify-center h-full">
              <p className="text-sm text-stone-600 dark:text-neutral-300">{t('chat.noMessages')}</p>
            </div>
          )}
        </div>

        <div className="flex-shrink-0 border-t border-stone-200 dark:border-neutral-800 px-4 py-3">
          <>
            {isNearLimit &&
              !isAtLimit &&
              isFreeTier &&
              shouldShowBanner('conversations-warning', 24 * 60 * 60 * 1000) && (
                <div className="mb-3">
                  <UpsellBanner
                    variant="warning"
                    title={t('chat.approachingLimit')}
                    message={t('chat.approachingLimitMsg').replace(
                      '{pct}',
                      String(Math.round(usagePct * 100))
                    )}
                    ctaLabel={t('chat.upgrade')}
                    onCtaClick={() => {
                      void openUrl(BILLING_DASHBOARD_URL);
                    }}
                    dismissible
                    onDismiss={() => dismissBanner('conversations-warning')}
                  />
                </div>
              )}
            {teamUsage && shouldShowBudgetCompletedMessage && (
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
                    {teamUsage.cycleBudgetUsd > 0
                      ? `${t('chat.weeklyLimitHit')}${teamUsage.cycleEndsAt ? ` ${t('chat.resets')} ${formatResetTime(teamUsage.cycleEndsAt)}.` : ''} ${t('chat.topUpToContinue')}`
                      : t('chat.budgetComplete')}
                  </p>
                </div>
                <button
                  onClick={() => {
                    void openUrl(BILLING_DASHBOARD_URL);
                  }}
                  className="flex-shrink-0 px-3 py-1.5 rounded-lg bg-coral-500 hover:bg-coral-400 text-white text-xs font-medium transition-colors">
                  {t('chat.topUp')}
                </button>
              </div>
            )}

            {/* Cycle usage pill. Backend PR #790 dropped rate-limit gating —
                  only budget-based pressure is surfaced here now. */}
            <div className="flex items-center justify-end gap-2 mb-2">
              {(isLoadingBudget || teamUsage) && (
                <div className="relative group">
                  {teamUsage ? (
                    <LimitPill label={t('chat.cycle')} usedPct={usagePct} />
                  ) : (
                    <span className="text-[10px] text-stone-400 dark:text-neutral-500 animate-pulse">
                      {t('common.loading')}
                    </span>
                  )}
                  {teamUsage && (
                    <div className="absolute bottom-full right-0 mb-2 hidden group-hover:block z-50">
                      <div className="bg-stone-900 text-white text-[10px] rounded-lg px-3 py-2 shadow-lg whitespace-nowrap space-y-1.5">
                        <div className="flex items-center justify-between gap-4">
                          <span className="text-stone-400">{t('chat.cycleSpent')}</span>
                          <span>
                            ${(teamUsage.cycleSpentUsd ?? 0).toFixed(2)} / $
                            {(teamUsage.cycleBudgetUsd ?? 0).toFixed(2)}
                          </span>
                        </div>
                        <div className="flex items-center justify-between gap-4">
                          <span className="text-stone-400">{t('chat.cycleRemaining')}</span>
                          <span>
                            ${(teamUsage.remainingUsd ?? 0).toFixed(2)} {t('chat.left')}
                            {teamUsage.cycleEndsAt && (
                              <span className="text-stone-400 dark:text-neutral-500 ml-1">
                                — {t('chat.resets')} {formatResetTime(teamUsage.cycleEndsAt)}
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
                className="text-xs text-stone-500 dark:text-neutral-400 hover:text-stone-700 dark:hover:text-neutral-200 dark:text-neutral-200 dark:hover:text-neutral-200 transition-colors ml-2">
                {t('common.dismiss')}
              </button>
            </div>
          )}

          {attachError && (
            <div className="flex items-center justify-between mb-2">
              <p className="text-xs text-coral-500" data-chat-send-error-code={attachError.code}>
                {attachError.message}
              </p>
              <button
                onClick={() => setAttachError(null)}
                className="text-xs text-stone-500 dark:text-neutral-400 hover:text-stone-700 dark:hover:text-neutral-200 transition-colors ml-2">
                {t('common.dismiss')}
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
                  sendError.code === 'voice_transcription' ||
                  sendError.code === 'tts_not_ready' ||
                  sendError.code === 'voice_synthesis') && (
                  <button
                    onClick={() => {
                      setSendError(null);
                      // STT/TTS provider settings live on the Voice panel
                      // since PR 2; the legacy local-model route was for
                      // back when speech assets were lumped with Ollama.
                      navigate('/settings/voice');
                    }}
                    className="text-xs text-primary-500 hover:text-primary-600 font-medium transition-colors">
                    {t('chat.setup')}
                  </button>
                )}
                <button
                  onClick={() => setSendError(null)}
                  className="text-xs text-stone-500 dark:text-neutral-400 hover:text-stone-700 dark:hover:text-neutral-200 dark:text-neutral-200 dark:hover:text-neutral-200 transition-colors">
                  {t('common.dismiss')}
                </button>
              </div>
            </div>
          )}

          {(() => {
            // Surface a parked ApprovalGate request for the shown thread just
            // above the composer, so it stays visible regardless of scroll.
            const approvalThreadId = selectedThreadId ?? activeThreadId;
            const pendingApproval = approvalThreadId
              ? pendingApprovalByThread[approvalThreadId]
              : undefined;
            return pendingApproval && approvalThreadId ? (
              <div className="mb-2">
                <ApprovalRequestCard threadId={approvalThreadId} approval={pendingApproval} />
              </div>
            ) : null;
          })()}

          {composer === 'mic-cloud' ? (
            <div className="flex flex-col items-center gap-3 py-1">
              <MicComposer
                // Without `!selectedThreadId`, a mic submit before a thread is
                // ready hits `handleSendMessage`'s early return and the
                // transcript is silently dropped — the user spoke into the void.
                disabled={composerInteractionBlocked || isSending || !selectedThreadId}
                onSubmit={text => handleSendMessage(text)}
                onError={message => setSendError(chatSendError('voice_transcription', message))}
                showDeviceSelector
                onSwitchToText={() => setComposerOverride('text')}
              />
            </div>
          ) : inputMode === 'text' ? (
            <div className="flex items-end gap-3">
              {/* Hidden file input for image attachment */}
              <input
                ref={fileInputRef}
                type="file"
                accept={ALLOWED_IMAGE_MIME_TYPES.join(',')}
                multiple
                className="hidden"
                onChange={e => {
                  void handleAttachFiles(e.target.files);
                  e.target.value = '';
                }}
              />
              <div className="relative flex flex-1 flex-col rounded-xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 transition-all focus-within:border-primary-500/50 focus-within:ring-1 focus-within:ring-primary-500/50">
                <AttachmentPreview
                  attachments={attachments}
                  onRemove={id => setAttachments(prev => prev.filter(a => a.id !== id))}
                  disabled={composerInteractionBlocked || isSending}
                />
                <div className="relative flex items-center justify-center">
                  <div
                    aria-hidden
                    className="pointer-events-none absolute inset-0 overflow-hidden whitespace-pre-wrap break-words px-4 py-2.5 text-sm leading-normal font-sans">
                    <span className="invisible">{inputValue}</span>
                    <span className="text-stone-500 dark:text-neutral-400/50">
                      {inlineCompletionSuffix}
                    </span>
                  </div>
                  <textarea
                    ref={textInputRef}
                    value={inputValue}
                    onChange={e => setInputValue(e.target.value)}
                    onCompositionStart={() => {
                      isComposingTextRef.current = true;
                    }}
                    onCompositionEnd={() => {
                      isComposingTextRef.current = false;
                    }}
                    onKeyDown={handleInputKeyDown}
                    placeholder={t('chat.typeMessage')}
                    rows={1}
                    disabled={composerInteractionBlocked || isSending}
                    className="relative z-10 w-full resize-none border-0 bg-transparent pl-4 pr-10 py-2.5 text-sm leading-normal whitespace-pre-wrap break-words font-sans text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 outline-none focus:outline-none focus-visible:outline-none focus:ring-0 focus-visible:ring-0 max-h-32 disabled:opacity-50 disabled:cursor-not-allowed"
                  />
                  {/* Voice input mic hidden per #717 (inputMode='voice' path retained). */}
                </div>
              </div>
              <button
                type="button"
                aria-label={t('chat.attachment.attach')}
                title={t('chat.attachment.attach')}
                onClick={() => fileInputRef.current?.click()}
                disabled={
                  composerInteractionBlocked ||
                  isSending ||
                  attachments.length >= ATTACHMENT_MAX_IMAGES
                }
                className="w-10 h-10 flex items-center justify-center rounded-full border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 text-stone-500 dark:text-neutral-400 hover:text-primary-500 dark:hover:text-primary-400 hover:border-primary-300 dark:hover:border-primary-700 transition-colors flex-shrink-0 disabled:opacity-40 disabled:cursor-not-allowed">
                <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13"
                  />
                </svg>
              </button>
              <button
                type="button"
                aria-label={t('mic.startRecording')}
                title={t('mic.startRecording')}
                onClick={() => setComposerOverride('mic-cloud')}
                disabled={composerInteractionBlocked || isSending}
                className="w-10 h-10 flex items-center justify-center rounded-full border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 text-stone-500 dark:text-neutral-400 hover:text-primary-500 dark:hover:text-primary-400 hover:border-primary-300 dark:hover:border-primary-700 transition-colors flex-shrink-0 disabled:opacity-40 disabled:cursor-not-allowed">
                <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M12 1a3 3 0 00-3 3v8a3 3 0 006 0V4a3 3 0 00-3-3z"
                  />
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M19 10v2a7 7 0 01-14 0v-2M12 19v4m-4 0h8"
                  />
                </svg>
              </button>
              <button
                data-testid="send-message-button"
                aria-label={t('chat.send')}
                title={t('chat.send')}
                onClick={() => {
                  void handleSendMessage();
                }}
                disabled={
                  (!inputValue.trim() && attachments.length === 0) ||
                  composerInteractionBlocked ||
                  isSending
                }
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
                className="w-10 h-10 flex items-center justify-center rounded-full border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 text-stone-500 dark:text-neutral-400 hover:text-stone-700 dark:hover:text-neutral-200 dark:text-neutral-200 dark:hover:text-neutral-200 hover:border-stone-300 dark:hover:border-neutral-700 transition-colors disabled:opacity-40"
                title={t('chat.switchToText')}>
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
                {isTranscribing
                  ? t('chat.transcribing')
                  : isRecording
                    ? t('chat.stopAndSend')
                    : t('chat.startTalking')}
              </button>
              <p className="text-xs text-stone-400 dark:text-neutral-500 truncate">
                {voiceStatus ??
                  (isPlayingReply && replyMode === 'voice'
                    ? t('chat.playingVoiceReply')
                    : canUseMicrophoneApi
                      ? t('chat.voiceHint')
                      : t('chat.micUnavailable'))}
              </p>
            </div>
          )}
        </div>
      </div>
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
