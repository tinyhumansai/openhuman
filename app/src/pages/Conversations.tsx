import { convertFileSrc } from '@tauri-apps/api/core';
import debugFactory from 'debug';
import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useLocation, useNavigate, useParams } from 'react-router-dom';

import { type ChatSendError, chatSendError } from '../chat/chatSendError';
import { checkPromptInjection, promptGuardMessage } from '../chat/promptInjectionGuard';
import ApprovalRequestCard from '../components/chat/ApprovalRequestCard';
import ArtifactCard from '../components/chat/ArtifactCard';
import ChatComposer from '../components/chat/ChatComposer';
import ChatFilesChip from '../components/chat/ChatFilesChip';
import ChatNewWindowHero from '../components/chat/ChatNewWindowHero';
import ComposerTokenStats from '../components/chat/ComposerTokenStats';
import IntegrationConnectCard from '../components/chat/IntegrationConnectCard';
import QueuedFollowups from '../components/chat/QueuedFollowups';
import SuperContextToggle from '../components/chat/SuperContextToggle';
import { whenSuperContextWriteSettled } from '../components/chat/superContextWrite';
import { ConfirmationModal } from '../components/intelligence/ConfirmationModal';
import { SidebarContent } from '../components/layout/shell/SidebarSlot';
import { settingsNavState } from '../components/settings/modal/settingsOverlay';
import UpsellBanner from '../components/upsell/UpsellBanner';
import { dismissBanner, shouldShowBanner } from '../components/upsell/upsellDismissState';
import MicComposer from '../features/human/MicComposer';
import { useStickToBottom } from '../hooks/useStickToBottom';
import { useUsageState } from '../hooks/useUsageState';
import {
  type Attachment,
  ATTACHMENT_MAX_FILES,
  ATTACHMENT_MAX_IMAGES,
  buildMessageWithAttachments,
  imageMarkerCost,
  parseMessageImages,
  validateAndReadFile,
} from '../lib/attachments';
import { useT } from '../lib/i18n/I18nContext';
import { trackEvent } from '../services/analytics';
import { applyOpenRouterFreeModels } from '../services/api/openrouterFreeModels';
import { subagentApi } from '../services/api/subagentApi';
import { threadApi } from '../services/api/threadApi';
import { fetchThreadTokenUsage } from '../services/api/threadUsageApi';
import {
  aiRegenerate,
  chatCancel,
  chatClearQueue,
  chatSend,
  useRustChat,
} from '../services/chatService';
import { callCoreRpc } from '../services/coreRpcClient';
import {
  loadAgentProfiles,
  selectActiveAgentProfileId,
  selectAgentProfile,
  selectAgentProfiles,
} from '../store/agentProfileSlice';
import {
  beginInferenceTurn,
  clearFollowupsForThread,
  clearRuntimeForThread,
  clearThreadSendPending,
  enqueueFollowup,
  fetchAndHydrateTurnState,
  hydrateThreadUsage,
  markSubagentCancelled,
  markThreadSendPending,
  type QueuedFollowup,
  registerParallelRequest,
  setTaskBoardForThread,
  setToolTimelineForThread,
  type ToolTimelineEntry,
} from '../store/chatRuntimeSlice';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import { selectSocketStatus } from '../store/socketSelectors';
import {
  addMessageLocal,
  clearThreadInferenceActive,
  createNewThread,
  deleteThread,
  loadThreadMessages,
  loadThreads,
  markThreadInferenceActive,
  persistReaction,
  setSelectedThread,
  THREAD_NOT_FOUND_MESSAGE,
  updateThreadTitle,
} from '../store/threadSlice';
import type { ConfirmationModal as ConfirmationModalType } from '../types/intelligence';
import type { ThreadMessage } from '../types/thread';
import { splitAgentMessageIntoBubbles } from '../utils/agentMessageBubbles';
import { chatThreadPath } from '../utils/chatRoutes';
import { CHAT_ATTACHMENTS_ENABLED } from '../utils/config';
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
import {
  AgentMessageBubble,
  AgentMessageText,
  BubbleMarkdown,
} from './conversations/components/AgentMessageBubble';
import { AgentProcessSourcePanel } from './conversations/components/AgentProcessSourcePanel';
import {
  BackgroundProcessesPanel,
  selectBackgroundProcesses,
} from './conversations/components/BackgroundProcessesPanel';
import { CitationChips, type MessageCitation } from './conversations/components/CitationChips';
import { PlanReviewCard } from './conversations/components/PlanReviewCard';
import { SubagentDrawer } from './conversations/components/SubagentDrawer';
import {
  ThreadGoalEditorPanel,
  ThreadGoalFooterTrigger,
  useThreadGoal,
} from './conversations/components/ThreadGoalChip';
import { ThreadTodoStrip } from './conversations/components/ThreadTodoStrip';
import { ToolTimelineBlock } from './conversations/components/ToolTimelineBlock';
import {
  evaluateComposerSend,
  getComposerBlockedSendFeedback,
  handleComposerSlashCommand,
} from './conversations/composerSendDecision';
import { useMemorySyncActive } from './conversations/hooks/useBackgroundActivity';
import {
  type AgentBubblePosition,
  buildAcceptedInlineCompletion,
  formatRelativeTime,
  formatResetTime,
  getInlineCompletionSuffix,
} from './conversations/utils/format';
import { GENERAL_TAB_VALUE, isThreadVisibleInTab } from './conversations/utils/threadFilter';

const CHAT_MODEL_HINT = 'hint:chat';
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
  /**
   * Project the thread list into the root sidebar's dynamic region even in the
   * `sidebar` variant. Page variant always projects it; this lets an embedded
   * instance (e.g. the Human page's right-rail chat) surface the user's threads
   * in the left sidebar while keeping the chat itself on the right. The list
   * and the chat share the same selection state, so clicking a thread switches
   * the embedded conversation.
   */
  projectThreadList?: boolean;
}

// Stable empty reference so the `activeThreadIds` selector returns the same
// object identity when the slice field is absent (narrow test stores),
// avoiding spurious re-renders.
const EMPTY_ACTIVE_THREADS: Record<string, true> = {};

// Stable empty reference for the queued-follow-ups map, so the selector keeps
// the same identity when the slice field is absent (narrow test stores).
const EMPTY_QUEUED_FOLLOWUPS: Record<string, QueuedFollowup[]> = {};

export function isComposerInteractionBlocked(args: {
  /** Whether the *currently selected* thread has an in-flight inference turn. */
  selectedThreadActive: boolean;
  rustChat: boolean;
}): boolean {
  return !args.rustChat || args.selectedThreadActive;
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

const Conversations = ({
  variant = 'page',
  composer: composerProp = 'text',
  projectThreadList = false,
}: ConversationsProps = {}) => {
  const [composerOverride, setComposerOverride] = useState<'mic-cloud' | 'text' | null>(null);
  const composer = composerOverride ?? composerProp;
  const { t } = useT();
  const dispatch = useAppDispatch();
  const navigate = useNavigate();
  const location = useLocation();
  const { threadId: routeThreadId } = useParams<{ threadId?: string }>();
  const shouldSyncChatRoute = variant === 'page' && location.pathname.startsWith('/chat');
  const { threads, selectedThreadId, messages, isLoadingMessages, messagesError } = useAppSelector(
    state => state.thread
  );
  // Optional-chain + default: narrow test stores may omit `activeThreadIds`.
  const activeThreadIds = useAppSelector(
    state => state.thread.activeThreadIds ?? EMPTY_ACTIVE_THREADS
  );
  // Per-thread inference tracking (parallel inference): the selected thread's
  // own in-flight state gates the composer; a turn running on a *different*
  // thread no longer locks this one. `firstActiveThreadId` is a best-effort
  // fallback for thread-scoped chips/panels when no thread is selected.
  const selectedThreadActive = selectedThreadId
    ? Boolean(activeThreadIds[selectedThreadId])
    : false;
  const firstActiveThreadId = Object.keys(activeThreadIds)[0] ?? null;

  // Thread-goal controller shared by the footer trigger (under the composer)
  // and the editor panel (above the composer).
  const threadGoal = useThreadGoal(selectedThreadId ?? null);

  const [inputValue, setInputValue] = useState('');
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [copiedMessageId, setCopiedMessageId] = useState<string | null>(null);
  // Sub-agent whose full live transcript is open in the drawer, keyed by the
  // owning timeline row's spawn `taskId`. Null when the drawer is closed.
  const [openSubagentTaskId, setOpenSubagentTaskId] = useState<string | null>(null);
  // Detached background sub-agents (spawn_async_subagent) panel visibility.
  const [showBackgroundProcesses, setShowBackgroundProcesses] = useState(false);
  // Whether the consolidated "Agent Process Source" panel is open (the full
  // agent-run timeline + visited sources for the current thread).
  const [showProcessSource, setShowProcessSource] = useState(false);
  // When the user clicks a step's "View details →", the Agent Process Source
  // panel is scoped to that single step. `null` = the whole-run overview
  // (opened by the bottom "View full agent process Source" link).
  const [scopedDetailEntryId, setScopedDetailEntryId] = useState<string | null>(null);
  const [inputMode, setInputMode] = useState<InputMode>('text');
  const [replyMode, setReplyMode] = useState<ReplyMode>('text');
  const [isRecording, setIsRecording] = useState(false);
  const [isTranscribing, setIsTranscribing] = useState(false);
  const [voiceStatus, setVoiceStatus] = useState<string | null>(null);
  const [isPlayingReply, setIsPlayingReply] = useState(false);
  // Measured height of the floating composer footer (page variant only). The
  // footer is `absolute`ly positioned over the scroll area, so the message list
  // needs matching bottom padding to keep its tail visible. Defaults to 128px
  // (the old static `pb-32`) so layout is unchanged until the ResizeObserver
  // reports a real height — and grows automatically when the queued-followups
  // panel, approval cards, or error banners expand the footer (#4268).
  const [composerFooterHeight, setComposerFooterHeight] = useState(128);
  // Thread-list filtering is fixed to the General bucket — the in-sidebar
  // General/Subconscious/Tasks chips were removed. Subconscious reflections and
  // task/worker threads have dedicated surfaces (Intelligence, Tasks board).
  const selectedLabel = GENERAL_TAB_VALUE;
  const [threadSearch, setThreadSearch] = useState('');
  const [inlineSuggestionValue, setInlineSuggestionValue] = useState('');
  const [sendError, setSendError] = useState<ChatSendError | null>(null);
  const [attachError, setAttachError] = useState<ChatSendError | null>(null);
  const [sendAdvisory, setSendAdvisory] = useState<string | null>(null);
  const [openRouterStatus, setOpenRouterStatus] = useState<'idle' | 'saving' | 'error'>('idle');
  // Threads whose send is mid-flight (dispatched locally, backend not yet
  // accepted). A Set so concurrent sends to different threads each track their
  // own pending state instead of clobbering a single slot.
  const [pendingSendingThreadIds, setPendingSendingThreadIds] = useState<ReadonlySet<string>>(
    () => new Set()
  );
  const addPendingSendingThread = useCallback(
    (threadId: string) => {
      // Mirror to Redux so global surfaces (e.g. the New Chat shortcut) can see
      // an in-flight send before any message/streaming state exists.
      dispatch(markThreadSendPending({ threadId }));
      setPendingSendingThreadIds(prev => {
        if (prev.has(threadId)) return prev;
        const next = new Set(prev);
        next.add(threadId);
        return next;
      });
    },
    [dispatch]
  );
  const removePendingSendingThread = useCallback(
    (threadId: string) => {
      dispatch(clearThreadSendPending({ threadId }));
      setPendingSendingThreadIds(prev => {
        if (!prev.has(threadId)) return prev;
        const next = new Set(prev);
        next.delete(threadId);
        return next;
      });
    },
    [dispatch]
  );
  const socketStatus = useAppSelector(selectSocketStatus);
  const agentProfiles = useAppSelector(selectAgentProfiles);
  const selectedAgentProfileId = useAppSelector(selectActiveAgentProfileId);
  // Optional chain because narrow test stores (e.g. Conversations.test
  // bootstraps without the locale slice) shouldn't crash here. `'en'`
  // matches the no-locale-directive branch in the core, so legacy
  // behaviour stays intact.
  const uiLocale = useAppSelector(state => state.locale?.current ?? 'en');
  const toolTimelineByThread = useAppSelector(state => state.chatRuntime.toolTimelineByThread);
  const processingByThread = useAppSelector(state => state.chatRuntime.processingByThread);
  const taskBoardByThread = useAppSelector(state => state.chatRuntime.taskBoardByThread);
  const inferenceStatusByThread = useAppSelector(
    state => state.chatRuntime.inferenceStatusByThread
  );
  const artifactsByThread = useAppSelector(state => state.chatRuntime.artifactsByThread);
  const pendingApprovalByThread = useAppSelector(
    state => state.chatRuntime.pendingApprovalByThread
  );
  const pendingPlanReviewByThread = useAppSelector(
    state => state.chatRuntime.pendingPlanReviewByThread
  );
  const streamingAssistantByThread = useAppSelector(
    state => state.chatRuntime.streamingAssistantByThread
  );
  // #4270: per-thread liveness counter bumped on each `inference_heartbeat`.
  // Watched by the silence-timer rearm effect so a long prefill / buffered
  // reasoning phase that streams no other progress still keeps the timer armed.
  const inferenceHeartbeatByThread = useAppSelector(
    state => state.chatRuntime.inferenceHeartbeatByThread
  );
  const parallelStreamsByThread = useAppSelector(
    state => state.chatRuntime.parallelStreamsByThread
  );
  const agentMessageViewMode = useAppSelector(
    state => state.theme?.agentMessageViewMode ?? 'bubbles'
  );
  // When ON, the verbose per-agent "Agentic task insights" timeline is hidden
  // from chat; a compact blinking "Processing" link (and the existing message
  // bubble loading) stand in for it, with the full run one click away in the
  // Agent Process Source side panel. See themeSlice.hideAgentInsights.
  const hideAgentInsights = useAppSelector(state => state.theme?.hideAgentInsights ?? false);
  const inferenceTurnLifecycleByThread = useAppSelector(
    state => state.chatRuntime.inferenceTurnLifecycleByThread
  );
  const queuedFollowupsByThread = useAppSelector(
    state => state.chatRuntime.queuedFollowupsByThread ?? EMPTY_QUEUED_FOLLOWUPS
  );
  const rustChat = useRustChat();
  const [reactionPickerMsgId, setReactionPickerMsgId] = useState<string | null>(null);
  // Inline thread-title rename in the sidebar thread list — keyed by the
  // thread id being edited (null = none) so any row can rename in place.
  const [editingThreadId, setEditingThreadId] = useState<string | null>(null);
  const [editTitleValue, setEditTitleValue] = useState('');
  const editTitleInputRef = useRef<HTMLInputElement>(null);
  const ignoreNextTitleBlurRef = useRef(false);

  const {
    teamUsage,
    isAtLimit,
    isNearLimit,
    isFreeTier,
    shouldShowBudgetCompletedMessage,
    usagePct,
    // #3767: gate on the tier for the selected chat mode — Quick runs on the
    // `chat` tier, Reasoning on the `reasoning` tier — so the credits prompt
    // reflects the mode the user actually picked.
  } = useUsageState(selectedAgentProfileId === 'reasoning' ? 'reasoning' : 'chat');
  const [deleteModal, setDeleteModal] = useState<ConfirmationModalType>({
    isOpen: false,
    title: '',
    message: '',
    onConfirm: () => {},
    onCancel: () => {},
  });
  const [resolvedModel, setResolvedModel] = useState<string | null>(null);
  // Whether the resolved model for the active profile accepts image input.
  // Managed tiers do; custom/BYOK models only when the user flagged them. Gates
  // the composer's image-attachment affordance (docs flow regardless). Resolved
  // against the non-attachment hint so the affordance is stable as you attach.
  const [modelSupportsVision, setModelSupportsVision] = useState(false);
  // Whether a vision-capable delegate (the `vision` sub-agent) is reachable.
  // When it is, an image may be attached and routed to that sub-agent even if
  // the active orchestrator model is non-vision — the orchestrator sees a text
  // placeholder and delegates the image to the vision sub-agent. Resolved from
  // the `vision` workload tier (vision-v1 on the managed backend, or the BYOK
  // model routed to the Vision workload).
  const [visionDelegateAvailable, setVisionDelegateAvailable] = useState(false);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const profile = agentProfiles.find(p => p.id === selectedAgentProfileId);
        // Resolve the actually-selected profile's model so `modelSupportsVision`
        // reflects the real tier, AND the vision workload so we know whether a
        // vision sub-agent can take the image. Documents are text-extracted so
        // any model handles them.
        const hint = profile?.modelOverride ?? CHAT_MODEL_HINT;
        const [res, visionRes] = await Promise.all([
          callCoreRpc<{ model: string; vision?: boolean }>({
            method: 'openhuman.inference_resolve_model',
            params: { hint },
          }),
          callCoreRpc<{ model: string; vision?: boolean }>({
            method: 'openhuman.inference_resolve_model',
            params: { hint: 'hint:vision' },
          }).catch(() => ({ model: '', vision: false })),
        ]);
        if (!cancelled) {
          setResolvedModel(res.model);
          setModelSupportsVision(res.vision === true);
          setVisionDelegateAvailable(visionRes.vision === true);
        }
      } catch {
        if (!cancelled) {
          setResolvedModel(null);
          setModelSupportsVision(false);
          setVisionDelegateAvailable(false);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [agentProfiles, selectedAgentProfileId]);

  const textInputRef = useRef<HTMLTextAreaElement>(null);
  const composerFooterRef = useRef<HTMLDivElement>(null);
  const isComposingTextRef = useRef(false);
  // Threads with an in-flight send, guarding against double-submit to the SAME
  // thread. Per-thread (a Set) so a send to thread B isn't blocked by an
  // in-flight send to thread A.
  const pendingSendsRef = useRef<Set<string>>(new Set());
  const mediaRecorderRef = useRef<MediaRecorder | null>(null);
  const mediaStreamRef = useRef<MediaStream | null>(null);
  const audioChunksRef = useRef<Blob[]>([]);
  const replyAudioRef = useRef<HTMLAudioElement | null>(null);
  const lastSpokenMessageIdRef = useRef<string | null>(null);
  const autocompleteDebounceRef = useRef<number | null>(null);
  const autocompleteRequestSeqRef = useRef(0);
  // Per-thread silence timers. Each in-flight turn gets its own 120s safety
  // timer keyed by thread id, so concurrent turns on different threads don't
  // share (and clobber) a single timeout.
  const sendingTimeoutsRef = useRef<Map<string, ReturnType<typeof setTimeout>>>(new Map());
  // Ref so the mount-time dictation event handler can call the latest send fn.
  const handleSendMessageRef = useRef<((text?: string) => Promise<void>) | null>(null);
  // Per-thread "turn signature": the last-seen tuple of progress-slice
  // references [inferenceStatus, streamingAssistant, toolTimeline, taskBoard]
  // for each thread that owns a live silence timer. Redux Toolkit (immer)
  // only produces new references for the thread whose slice actually changed,
  // so comparing references lets the rearm effect (a) detect a turn completing
  // (status defined → undefined) and (b) rearm a thread's timer ONLY when that
  // thread's own state changed — unrelated threads' activity must not keep a
  // foreground turn's timer alive.
  const turnSignatureByThreadRef = useRef<Map<string, readonly unknown[]>>(new Map());

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
    if (shouldSyncChatRoute) {
      debug('[chat][route] created thread thread=%s navigate=true', thread.id);
      navigate(chatThreadPath(thread.id));
    } else {
      debug('[chat][route] created thread thread=%s navigate=false', thread.id);
    }
  };

  const handleUseOpenRouterFree = async () => {
    setOpenRouterStatus('saving');
    try {
      await applyOpenRouterFreeModels();
      setOpenRouterStatus('idle');
    } catch (err) {
      console.warn('[chat] applyOpenRouterFreeModels failed', err);
      setOpenRouterStatus('error');
    }
  };

  const handleStartEditTitle = (threadId: string) => {
    const thr = threads.find(t => t.id === threadId);
    debug('[chat] thread rename: start thread=%s', threadId);
    setEditTitleValue(thr?.title ?? '');
    ignoreNextTitleBlurRef.current = true;
    setEditingThreadId(threadId);
    const scheduleSelect = window.requestAnimationFrame ?? window.setTimeout;
    scheduleSelect(() => {
      editTitleInputRef.current?.select();
      ignoreNextTitleBlurRef.current = false;
    });
  };

  const handleCommitTitle = (threadId: string) => {
    const trimmed = editTitleValue.trim();
    setEditingThreadId(null);
    // Title length only — never log the title text itself (may carry PII).
    if (!threadId || !trimmed) {
      debug('[chat] thread rename: commit skipped thread=%s empty=%s', threadId, !trimmed);
      return;
    }
    const currentTitle = threads.find(t => t.id === threadId)?.title?.trim();
    if (trimmed === currentTitle) {
      debug('[chat] thread rename: commit skipped thread=%s (unchanged)', threadId);
      return;
    }
    debug('[chat] thread rename: commit thread=%s len=%d', threadId, trimmed.length);
    void dispatch(updateThreadTitle({ threadId, title: trimmed }))
      .unwrap()
      .then(() => debug('[chat] thread rename: committed thread=%s', threadId))
      .catch(err =>
        debug(
          '[chat] thread rename: failed thread=%s err=%s',
          threadId,
          err instanceof Error ? err.message : String(err)
        )
      );
  };

  const handleSelectAgentProfile = async (profileId: string) => {
    try {
      await dispatch(selectAgentProfile(profileId)).unwrap();
    } catch (error) {
      debug('agent profile select failed: %o', error);
    }
  };

  // Seed the composer footer with the selected thread's persisted token/cost
  // usage (read back from its session transcripts) so the totals reflect prior
  // turns instead of starting at zero. Best-effort; live turns accumulate on top
  // via recordChatTurnUsage and a brand-new thread (hasUsage=false) is left as-is.
  useEffect(() => {
    if (!selectedThreadId) return;
    let cancelled = false;
    void fetchThreadTokenUsage(selectedThreadId)
      .then(u => {
        if (cancelled || !u.hasUsage) return;
        dispatch(
          hydrateThreadUsage({
            threadId: u.threadId,
            inputTokens: u.inputTokens,
            outputTokens: u.outputTokens,
            cachedTokens: u.cachedInputTokens,
            costUsd: u.costUsd,
            turns: u.turnCount,
            contextWindow: u.contextWindow,
            lastTurnInputTokens: u.lastTurnInputTokens,
            lastTurnOutputTokens: u.lastTurnOutputTokens,
            subAgents: u.subagents,
          })
        );
      })
      .catch(() => {
        /* best-effort seed; the footer still fills from live turns */
      });
    return () => {
      cancelled = true;
    };
  }, [selectedThreadId, dispatch]);

  useEffect(() => {
    let cancelled = false;

    void dispatch(loadThreads())
      .unwrap()
      .then(data => {
        if (cancelled) return;
        // Match the sidebar's default General filter here so initial/resume
        // selection can't auto-pick a thread hidden by the selected tab.
        const visibleThreads = data.threads.filter(t => isThreadVisibleInTab(t, GENERAL_TAB_VALUE));
        // An explicit "open this session" intent (e.g. View work from the Agent
        // Tasks board) wins over passive resume — and bypasses the General-tab
        // visibility filter so a task-labelled session thread can actually be
        // opened (the resume default below only considers General threads).
        const openThreadId =
          routeThreadId ?? (location.state as { openThreadId?: string } | null)?.openThreadId;
        const openThread = openThreadId ? data.threads.find(t => t.id === openThreadId) : undefined;
        if (openThread) {
          // An explicit open intent (e.g. View work from the Tasks board) opens
          // the thread in the main pane directly; the thread list itself stays
          // filtered to General.
          dispatch(setSelectedThread(openThread.id));
          void dispatch(loadThreadMessages(openThread.id));
          debug('[chat][route] opened requested thread thread=%s', openThread.id);
          return;
        }
        if (openThreadId) {
          debug('[chat][route] requested thread not found thread=%s; falling back', openThreadId);
          navigate('/chat', { replace: true });
          return;
        }
        // Restore the thread the user last had open — persisted across reloads
        // via redux-persist on the `thread` slice, and kept in-memory across
        // in-app navigation — whenever it still exists server-side. This must
        // run BEFORE the General-only default below: a non-General active
        // session (task / worker / subconscious / meeting) is filtered out of
        // `visibleThreads`, so without this branch, navigating away from the
        // Chat tab and back would drop the active thread and either resume an
        // unrelated General thread or spawn a fresh chat — losing the
        // conversation the user was in (#chat-tab-active-thread).
        const persistedThread = selectedThreadId
          ? data.threads.find(t => t.id === selectedThreadId)
          : undefined;
        if (persistedThread) {
          dispatch(setSelectedThread(persistedThread.id));
          void dispatch(loadThreadMessages(persistedThread.id));
          debug('[chat][route] restored active thread thread=%s', persistedThread.id);
          return;
        }
        // Default landing is a fresh "new window" (the merged Home surface) —
        // we no longer resume the last conversation on open. Reuse an existing
        // empty thread if one is lying around so repeated opens don't pile up
        // blank threads; otherwise create a new one. Past conversations stay
        // reachable from the thread list (clicking one selects it directly).
        const emptyThread = visibleThreads.find(t => (t.messageCount ?? 0) === 0);
        if (emptyThread) {
          dispatch(setSelectedThread(emptyThread.id));
          void dispatch(loadThreadMessages(emptyThread.id));
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
  }, [dispatch, routeThreadId]);

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

  const { containerRef: messagesContainerRef, endRef: messagesEndRef } = useStickToBottom(
    messages,
    selectedThreadId,
    location.pathname
  );

  useEffect(() => {
    const onDictationInsert = (event: Event) => {
      const customEvent = event as CustomEvent<{ text?: string; autoSend?: boolean }>;
      const text = customEvent.detail?.text?.trim();
      if (!text) return;

      customEvent.preventDefault();

      // When autoSend is set (hotkey dictation), dispatch the transcript directly
      // to the agent without going through the text composer.
      if (customEvent.detail?.autoSend) {
        void handleSendMessageRef.current?.(text);
        return;
      }

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

  const clearSilenceTimer = useCallback((threadId: string) => {
    const existing = sendingTimeoutsRef.current.get(threadId);
    if (existing) {
      clearTimeout(existing);
      sendingTimeoutsRef.current.delete(threadId);
    }
  }, []);

  const armSilenceTimer = (threadId: string) => {
    clearSilenceTimer(threadId);
    const timeout = setTimeout(() => {
      debug(`armSilenceTimer: no inference signal for 120s — clearing runtime (${threadId})`);
      setSendError(chatSendError('safety_timeout', t('chat.safetyTimeout')));
      dispatch(clearRuntimeForThread({ threadId }));
      dispatch(clearThreadInferenceActive(threadId));
      sendingTimeoutsRef.current.delete(threadId);
      // Reset so the NEXT send to this thread starts from a clean baseline —
      // otherwise the rearm effect could read this turn's last signature as a
      // stale "previous" and mis-handle the next send's first signal.
      turnSignatureByThreadRef.current.delete(threadId);
      pendingSendsRef.current.delete(threadId);
      removePendingSendingThread(threadId);
    }, 120_000);
    sendingTimeoutsRef.current.set(threadId, timeout);
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
  // Rearm each live silence timer when its OWN thread shows progress, and drop
  // it when that thread's turn completes. With parallel inference several
  // timers may be live at once, so we iterate every thread that currently owns
  // a timer. Per-thread reference comparison (see `turnSignatureByThreadRef`)
  // ensures an unrelated thread's activity does NOT rearm this thread's timer,
  // while still catching pure-text streams and sub-agent tool/board activity
  // that bump the other slices without re-emitting a top-level status.
  //
  // The done-transition (status defined → undefined) is detected per thread to
  // distinguish "turn just finished (chat_done / chat_error)" from "status
  // never set yet" — the Send handler dispatches `setToolTimelineForThread([])`
  // immediately after arming, firing this effect before any status publishes.
  useEffect(() => {
    for (const threadId of Array.from(sendingTimeoutsRef.current.keys())) {
      const current = [
        inferenceStatusByThread[threadId],
        streamingAssistantByThread[threadId],
        toolTimelineByThread[threadId],
        taskBoardByThread[threadId],
        // #4270: liveness beat. Kept LAST so the done-transition probe on
        // `current[0]` (status) is unaffected; a beat alone still flips the
        // `changed` check and rearms the timer through a silent reasoning phase.
        inferenceHeartbeatByThread[threadId],
      ] as const;
      const previous = turnSignatureByThreadRef.current.get(threadId);
      const status = current[0];
      const previousStatus = previous?.[0];
      if (status === undefined && previousStatus !== undefined) {
        clearSilenceTimer(threadId);
        turnSignatureByThreadRef.current.delete(threadId);
        continue;
      }
      const changed = !previous || previous.some((value, index) => value !== current[index]);
      if (!changed) continue;
      turnSignatureByThreadRef.current.set(threadId, current);
      armSilenceTimer(threadId);
    }
    // armSilenceTimer / clearSilenceTimer are stable (refs + dispatch);
    // depending on the progress maps rearms live timers on every signal.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    inferenceStatusByThread,
    streamingAssistantByThread,
    toolTimelineByThread,
    taskBoardByThread,
    inferenceHeartbeatByThread,
  ]);

  useEffect(() => {
    if (
      !isTauri() ||
      !rustChat ||
      inputMode !== 'text' ||
      selectedThreadActive ||
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
  }, [selectedThreadActive, inputValue, inputMode, rustChat]);

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

  const handleAttachFiles = async (files: FileList | File[] | null) => {
    if (!files) return;
    let acceptedFileCount = attachments.filter(attachment => attachment.kind === 'file').length;
    // Images and videos share one image-marker budget (video = its frames), so
    // track consumed markers rather than per-kind counts.
    let acceptedImageMarkers = attachments.reduce(
      (sum, attachment) => sum + imageMarkerCost(attachment.kind),
      0
    );
    for (const file of Array.from(files)) {
      const result = await validateAndReadFile(
        file,
        acceptedImageMarkers,
        acceptedFileCount,
        // Allow images AND video when the active model is vision-capable OR a
        // vision sub-agent can take it (orchestrator delegates the image/frames
        // onward). Video is sampled into still frames that ride the same path.
        modelSupportsVision || visionDelegateAvailable
      );
      if ('error' in result) {
        const { error } = result;
        if (error.code === 'image_not_supported') {
          setAttachError(
            chatSendError('attachment_invalid', t('chat.attachment.imageNotSupported'))
          );
        } else if (error.code === 'video_not_supported') {
          setAttachError(
            chatSendError('attachment_invalid', t('chat.attachment.videoNotSupported'))
          );
        } else if (error.code === 'too_many') {
          // image/video share the image-marker budget → tooMany; files separate.
          const key =
            error.kind === 'file' ? 'chat.attachment.tooManyFiles' : 'chat.attachment.tooMany';
          setAttachError(
            chatSendError('attachment_invalid', t(key).replace('{max}', String(error.max)))
          );
        } else if (error.code === 'too_large') {
          const maxMb = (error.maxBytes / (1024 * 1024)).toFixed(0);
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
      if (result.attachment.kind === 'file') {
        acceptedFileCount++;
      } else {
        acceptedImageMarkers += imageMarkerCost(result.attachment.kind);
      }
      setAttachments(prev => [...prev, result.attachment]);
    }
  };

  const handleSendMessage = async (text?: string) => {
    // Guard double-submit to the SAME thread only; a send to another thread
    // may proceed concurrently.
    if (selectedThreadId && pendingSendsRef.current.has(selectedThreadId)) return;

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
    pendingSendsRef.current.add(sendingThreadId);
    addPendingSendingThread(sendingThreadId);
    // If the user just flipped the Super Context toggle, make sure that config
    // write has landed before the core builds this thread's session (which
    // reads `context.super_context_enabled`). Done AFTER the duplicate-send
    // guard above is set so this await can't open a check→add race for rapid
    // repeat clicks. Resolves instantly when nothing is pending.
    await whenSuperContextWriteSettled();
    const pendingAttachments = attachments.slice();
    const modelOverride =
      agentProfiles.find(p => p.id === selectedAgentProfileId)?.modelOverride ?? CHAT_MODEL_HINT;
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
              attachmentKinds: pendingAttachments.map(a => a.kind),
              attachmentDataUris: pendingAttachments
                .filter(a => a.kind === 'image')
                .map(a => a.previewUri ?? a.dataUri),
              // Poster (first frame) per attachment, index-aligned with
              // attachmentKinds — only video entries carry one; others null.
              attachmentPosters: pendingAttachments.map(a =>
                a.kind === 'video' ? (a.previewUri ?? a.dataUri) : null
              ),
              attachmentCompressed: pendingAttachments.map(a => a.compressed),
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
        pendingSendsRef.current.delete(sendingThreadId);
        removePendingSendingThread(sendingThreadId);
        return;
      }
      const msg = error instanceof Error ? error.message : String(error);
      setSendError(chatSendError('cloud_send_failed', msg));
      pendingSendsRef.current.delete(sendingThreadId);
      removePendingSendingThread(sendingThreadId);
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
    turnSignatureByThreadRef.current.delete(sendingThreadId);
    armSilenceTimer(sendingThreadId);
    dispatch(setToolTimelineForThread({ threadId: sendingThreadId, entries: [] }));
    dispatch(beginInferenceTurn({ threadId: sendingThreadId }));
    dispatch(markThreadInferenceActive(sendingThreadId));

    // ── Cloud socket path ─────────────────────────────────────────────────────
    // Always route primary chat through the cloud backend via socket.
    // Local model (Ollama) is used only for supplementary features
    // (auto-react, autocomplete, etc.) — never as a primary chat path.
    try {
      await chatSend({
        threadId: sendingThreadId,
        message: messageText,
        model: modelOverride,
        profileId: selectedAgentProfileId,
        locale: uiLocale,
      });
      trackEvent('chat_message_sent');
      // Backend accepted the send; lifecycle ('started' → 'streaming') now
      // owns the `isSending` UI lock. Release the pending guard so the next
      // user turn isn't blocked by a stale ref/state.
      pendingSendsRef.current.delete(sendingThreadId);
      removePendingSendingThread(sendingThreadId);

      // Active-thread reset happens in the global ChatRuntimeProvider events.
    } catch (err) {
      // Chat loop errors are emitted via socket events; this catch handles emit-level failures.
      clearSilenceTimer(sendingThreadId);
      turnSignatureByThreadRef.current.delete(sendingThreadId);
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
      dispatch(clearThreadInferenceActive(sendingThreadId));
      pendingSendsRef.current.delete(sendingThreadId);
      removePendingSendingThread(sendingThreadId);
    }
  };

  handleSendMessageRef.current = handleSendMessage;

  // Send a PARALLEL (forked) turn on the selected thread — runs concurrently
  // with the in-flight turn instead of interrupting it (queue_mode 'parallel').
  // Kept separate from `handleSendMessage` so it never touches the primary
  // turn's lifecycle (silence timer, active marker, pending guard); the forked
  // turn streams into its own lane (registered via `registerParallelRequest`)
  // and renders as an interleaved branch bubble.
  const handleSendParallel = async (text?: string) => {
    if (!rustChat || !selectedThreadId) return;
    const threadId = selectedThreadId;
    const normalized = (text ?? inputValue).trim();
    if (!normalized && attachments.length === 0) return;

    const pendingAttachments = attachments.slice();
    const modelOverride =
      agentProfiles.find(p => p.id === selectedAgentProfileId)?.modelOverride ?? CHAT_MODEL_HINT;
    const messageText = buildMessageWithAttachments(normalized, pendingAttachments);
    const userMessage: ThreadMessage = {
      id: `msg_${globalThis.crypto.randomUUID()}`,
      content: normalized,
      type: 'text',
      extraMetadata:
        pendingAttachments.length > 0
          ? {
              attachmentCount: pendingAttachments.length,
              attachmentNames: pendingAttachments.map(a => a.file.name),
              attachmentKinds: pendingAttachments.map(a => a.kind),
              attachmentDataUris: pendingAttachments
                .filter(a => a.kind === 'image')
                .map(a => a.previewUri ?? a.dataUri),
              // Poster (first frame) per attachment, index-aligned with
              // attachmentKinds — only video entries carry one; others null.
              attachmentPosters: pendingAttachments.map(a =>
                a.kind === 'video' ? (a.previewUri ?? a.dataUri) : null
              ),
              attachmentCompressed: pendingAttachments.map(a => a.compressed),
              parallelBranch: true,
            }
          : { parallelBranch: true },
      sender: 'user',
      createdAt: new Date().toISOString(),
    };

    try {
      await dispatch(addMessageLocal({ threadId, message: userMessage })).unwrap();
    } catch (error) {
      if (error === THREAD_NOT_FOUND_MESSAGE) return;
      const msg = error instanceof Error ? error.message : String(error);
      setSendError(chatSendError('cloud_send_failed', msg));
      return;
    }

    setInputValue('');
    setAttachments([]);
    setSendError(null);

    try {
      const requestId = await chatSend({
        threadId,
        message: messageText,
        model: modelOverride,
        profileId: selectedAgentProfileId,
        locale: uiLocale,
        queueMode: 'parallel',
      });
      if (requestId) {
        dispatch(registerParallelRequest({ threadId, requestId }));
      }
      trackEvent('chat_parallel_message_sent');
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setSendError(chatSendError('cloud_send_failed', msg));
    }
  };

  // Queue a FOLLOW-UP on the selected thread while a turn is streaming
  // (queue_mode 'followup'): the backend sends it as a fresh turn once the
  // current turn finishes. We do NOT insert it into the transcript now —
  // appending it mid-stream would persist it BEFORE the in-flight assistant
  // reply (the conversation store is an append log), so the prompt would show
  // out of order on reload. Instead we record a queued-follow-up pill; the pill
  // is flushed into the transcript (persisted, in order, after the assistant
  // reply) when the turn ends — see `ChatRuntimeProvider`'s done/error paths.
  const handleSendFollowup = async (text?: string) => {
    if (!rustChat || !selectedThreadId) return;
    const threadId = selectedThreadId;
    const normalized = (text ?? inputValue).trim();
    const pendingAttachments = attachments.slice();
    if (!normalized && pendingAttachments.length === 0) return;

    const modelOverride =
      agentProfiles.find(p => p.id === selectedAgentProfileId)?.modelOverride ?? CHAT_MODEL_HINT;
    const messageText = buildMessageWithAttachments(normalized, pendingAttachments);
    // Build the full user message exactly like a normal send (content +
    // attachment metadata) so the follow-up persists identically when it is
    // flushed into the transcript on turn end. Guard `crypto.randomUUID` like
    // the rest of the codebase (threadSlice) for runtimes that lack it.
    const messageId = `msg_${
      globalThis.crypto?.randomUUID
        ? globalThis.crypto.randomUUID()
        : `${Date.now()}-${Math.random().toString(36).slice(2)}`
    }`;
    const followupMessage: ThreadMessage = {
      id: messageId,
      content: normalized,
      type: 'text',
      extraMetadata:
        pendingAttachments.length > 0
          ? {
              attachmentCount: pendingAttachments.length,
              attachmentNames: pendingAttachments.map(a => a.file.name),
              attachmentKinds: pendingAttachments.map(a => a.kind),
              attachmentDataUris: pendingAttachments
                .filter(a => a.kind === 'image')
                .map(a => a.previewUri ?? a.dataUri),
              // Poster (first frame) per attachment, index-aligned with
              // attachmentKinds — only video entries carry one; others null.
              attachmentPosters: pendingAttachments.map(a =>
                a.kind === 'video' ? (a.previewUri ?? a.dataUri) : null
              ),
              attachmentCompressed: pendingAttachments.map(a => a.compressed),
            }
          : {},
      sender: 'user',
      createdAt: new Date().toISOString(),
    };
    // Never render a blank pill for an attachments-only follow-up: fall back to
    // the attachment file names as the label.
    const label = normalized || pendingAttachments.map(a => a.file.name).join(', ');

    setSendError(null);
    setAttachError(null);

    try {
      await chatSend({
        threadId,
        message: messageText,
        model: modelOverride,
        profileId: selectedAgentProfileId,
        locale: uiLocale,
        queueMode: 'followup',
      });
      // Only clear the composer once the backend has accepted the queue, so a
      // failed send leaves the user's draft + attachments intact to retry.
      setInputValue('');
      setAttachments([]);
      dispatch(enqueueFollowup({ threadId, message: followupMessage, label }));
      trackEvent('chat_followup_queued');
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setSendError(chatSendError('cloud_send_failed', msg));
    }
  };

  // Dismiss every queued follow-up for the selected thread. Clear the backend
  // run-queue FIRST and only drop the local pills if it succeeded — on failure
  // the backend still holds (and will dispatch) the follow-ups, so keep the
  // pills and surface the error rather than falsely showing them removed.
  const handleClearQueuedFollowups = async () => {
    if (!selectedThreadId) return;
    const threadId = selectedThreadId;
    const dropped = await chatClearQueue(threadId);
    if (dropped === null) {
      setSendError(chatSendError('cloud_send_failed', t('chat.queuedFollowups.clearFailed')));
      return;
    }
    dispatch(clearFollowupsForThread({ threadId }));
  };

  // The composer's Send button (and plain Enter) route to a queued follow-up
  // while the selected thread is streaming, otherwise to a normal send.
  const handleComposerSend = (text?: string): Promise<void> =>
    selectedThreadActive ? handleSendFollowup(text) : handleSendMessage(text);

  // Cancel the in-flight turn for the selected thread. Shared by the in-composer
  // Stop button (text mode) and the footer Cancel control (mic-cloud / voice
  // modes) so the cancel path lives in one place.
  const handleStopGeneration = useCallback(() => {
    if (selectedThreadId) void chatCancel(selectedThreadId);
  }, [selectedThreadId]);

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
    if (!rustChat || selectedThreadActive || isTranscribing) return;
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

    // Cmd/Ctrl+Enter sends a PARALLEL branch when the selected thread already
    // has a turn in flight (otherwise it behaves like a normal send).
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      if (selectedThreadActive) {
        void handleSendParallel();
      } else {
        void handleSendMessage();
      }
      return;
    }

    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      // While the selected thread is streaming, a plain Enter queues a
      // follow-up (sent after the current turn) instead of being blocked.
      if (selectedThreadActive) {
        void handleSendFollowup();
      } else {
        void handleSendMessage();
      }
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
  const selectedThreadProcessing = selectedThreadId
    ? (processingByThread[selectedThreadId] ?? [])
    : [];
  // Detached background sub-agents (mode === 'async') spawned in this thread.
  const backgroundProcesses = useMemo(
    () => selectBackgroundProcesses(selectedThreadToolTimeline),
    [selectedThreadToolTimeline]
  );
  const runningBackgroundCount = backgroundProcesses.filter(p => p.status === 'running').length;
  // Poll-free live signal: lights the badge when memories are syncing even if
  // no sub-agent is running and the panel is closed.
  const memorySyncActive = useMemorySyncActive();
  // Re-derive the open subagent's live activity (and its row status) from the
  // timeline on every render so the drawer streams token-by-token as
  // subagent_text_delta / subagent_thinking_delta events land in Redux.
  const openSubagentEntry = openSubagentTaskId
    ? selectedThreadToolTimeline.find(entry => entry.subagent?.taskId === openSubagentTaskId)
    : undefined;
  const selectedTaskBoard = selectedThreadId ? (taskBoardByThread[selectedThreadId] ?? null) : null;
  const hasTaskBoard = Boolean(selectedTaskBoard?.cards.length);
  // A plan the orchestrator parked for interactive review (request_plan_review
  // gate). When present, the PlanReviewCard renders above the composer and
  // resolves the parked turn; the todo strip stays read-only progress.
  const pendingPlanReview = selectedThreadId
    ? (pendingPlanReviewByThread[selectedThreadId] ?? null)
    : null;
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
  // Live streams for concurrent parallel (forked) turns on the selected thread,
  // rendered as separate interleaved branch bubbles.
  const selectedParallelStreams = selectedThreadId
    ? Object.values(parallelStreamsByThread[selectedThreadId] ?? {})
    : [];
  const inlineCompletionSuffix = getInlineCompletionSuffix(inputValue, inlineSuggestionValue);
  // Blocks all composer interaction while a turn is in-flight or Rust chat is unavailable.
  // isSending: the *selected* thread is in-flight (drives selected-thread UI only).
  const composerInteractionBlocked = isComposerInteractionBlocked({
    selectedThreadActive,
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
    (pendingSendingThreadIds.has(selectedThreadId) ||
      inferenceTurnLifecycleByThread[selectedThreadId] === 'started' ||
      inferenceTurnLifecycleByThread[selectedThreadId] === 'streaming')
  );
  const shouldRenderTimelineBeforeLatestAgentMessage =
    selectedThreadToolTimeline.length > 0 && !isSending && Boolean(latestVisibleAgentMessage);

  // Anchor the "Agentic task insights" panel right after the latest turn's user
  // message — processing happens *before* the answer, so it reads above the
  // result (for both the live streaming preview and the settled agent bubbles).
  // Anchoring on the user message (not the first/last agent message) avoids the
  // multi-agent-message split from issue #3717.
  const lastUserMessageId = [...visibleMessages].reverse().find(m => m.sender === 'user')?.id;

  // The insights panel (timeline + "View full agent process Source" opener),
  // built once and rendered inline above the latest answer. `null` when there
  // are no recorded steps for the thread.
  // Open the Agent Process Source panel scoped to one step, or to the whole run.
  const openScopedDetail = (entry: ToolTimelineEntry) => {
    setScopedDetailEntryId(entry.id);
    setShowProcessSource(true);
  };
  const openWholeRunSource = () => {
    setScopedDetailEntryId(null);
    setShowProcessSource(true);
  };
  const scopedDetailEntry =
    scopedDetailEntryId != null
      ? selectedThreadToolTimeline.find(e => e.id === scopedDetailEntryId)
      : undefined;

  const agentInsights =
    // Render when there are tool steps OR a persisted reasoning/narration
    // transcript. A tool-less turn (the agent only thinks/narrates, no tool
    // calls) has an empty timeline but still persists thoughts — without the
    // transcript guard those thoughts would be unreachable.
    selectedThreadToolTimeline.length > 0 || selectedThreadProcessing.length > 0 ? (
      <>
        {hideAgentInsights ? (
          // "Hide agent thinking" is ON: suppress the verbose step rows.
          // While in flight, surface a compact blinking "Processing" link; once
          // settled the "View full agent process Source" opener below takes
          // over (so only render this fallback when that opener won't).
          isSending ? (
            <button
              type="button"
              onClick={openWholeRunSource}
              data-testid="agent-processing-link"
              className="flex items-center gap-1.5 px-1 py-1 text-[11px] font-medium text-primary-600 hover:underline dark:text-primary-300">
              <span className="inline-block w-1.5 h-1.5 rounded-full bg-primary-400 animate-pulse" />
              <span>{t('conversations.agentTaskInsights.processing')} →</span>
            </button>
          ) : !shouldRenderTimelineBeforeLatestAgentMessage ? (
            <button
              type="button"
              onClick={openWholeRunSource}
              data-testid="agent-process-source-fallback"
              className="px-1 text-[11px] font-medium text-primary-600 hover:underline dark:text-primary-300">
              {t('conversations.agentTaskInsights.viewProcessSource')} →
            </button>
          ) : null
        ) : selectedThreadToolTimeline.length > 0 ? (
          <ToolTimelineBlock
            entries={selectedThreadToolTimeline}
            onViewDetails={openScopedDetail}
            onViewWholeRun={openWholeRunSource}
            liveResponse={selectedStreamingAssistant?.content}
          />
        ) : (
          // Transcript-only turn: reasoning/narration was streamed but no tool
          // calls were made, so the inline step timeline is empty. The thoughts
          // are still persisted — surface a standalone opener (matching the
          // settled insights header) so the full-run panel stays reachable.
          <button
            type="button"
            onClick={openWholeRunSource}
            data-testid="view-process-source"
            className="flex items-center gap-1.5 px-1 py-1 text-left">
            <span className="text-[13px] font-medium text-content-muted">
              {t('conversations.agentTaskInsights.title')}
            </span>
            <span className="text-[13px] font-medium text-primary-600 dark:text-primary-300">
              →
            </span>
          </button>
        )}
        {/* "View full agent process Source" — only needed in the hidden-insights
            settled state; when the timeline is visible the link lives in its
            header (ToolTimelineBlock onViewWholeRun). */}
        {shouldRenderTimelineBeforeLatestAgentMessage && hideAgentInsights && (
          <button
            type="button"
            onClick={openWholeRunSource}
            data-testid="view-process-source"
            className="px-1 text-[11px] font-medium text-primary-600 hover:underline dark:text-primary-300">
            {t('conversations.agentTaskInsights.viewProcessSource')} →
          </button>
        )}
      </>
    ) : null;

  const filteredThreads = useMemo(() => {
    return threads.filter(t => isThreadVisibleInTab(t, selectedLabel));
  }, [threads, selectedLabel]);

  const sortedThreads = useMemo(() => {
    return [...filteredThreads].sort(
      (a, b) => new Date(b.lastMessageAt).getTime() - new Date(a.lastMessageAt).getTime()
    );
  }, [filteredThreads]);

  // Free-text search over the thread sidebar — filters the visible list by
  // title (mirrors the settings sidebar search).
  const visibleThreads = useMemo(() => {
    const q = threadSearch.trim().toLowerCase();
    if (!q) return sortedThreads;
    return sortedThreads.filter(thread => (thread.title ?? '').toLowerCase().includes(q));
  }, [sortedThreads, threadSearch]);

  const isSidebar = variant === 'sidebar';
  // "New window" = the merged Home surface: a page-variant chat whose selected
  // thread has no messages yet. We show the greeting + banners hero above a
  // centered composer; the moment the first message lands, hasVisibleMessages
  // flips true and this collapses back to the normal conversation layout.
  const isNewWindow =
    !isSidebar && !isLoadingMessages && !messagesError && !hasVisibleMessages && !hasTaskBoard;

  // Track the floating composer footer's height so the message list can reserve
  // matching bottom padding. In the page variant the footer is absolutely
  // positioned over the scroll area, so a static padding (the old `pb-32`) gets
  // overrun whenever the footer grows — most visibly when the "Queued
  // follow-ups" panel appears mid-reply, hiding the tail of the response
  // (#4268). The sidebar variant lays the composer out in normal flow and never
  // overlaps, so we skip the observer there and keep its `pb-4`.
  useEffect(() => {
    if (isSidebar) return;
    const el = composerFooterRef.current;
    if (!el) return;
    const measure = () => {
      const next = Math.round(el.getBoundingClientRect().height);
      if (next > 0) setComposerFooterHeight(next);
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, [isSidebar, selectedThreadId]);

  // Stable title resolver used by both the sidebar thread list and the header.
  const resolveThreadDisplayTitle = (threadId: string | null): string => {
    if (!threadId) return t('chat.selectThread');
    const thr = threads.find(th => th.id === threadId);
    return thr?.title ?? t('chat.selectThread');
  };

  // Resolve the parent of the currently-selected thread, if any. Used to
  // render the back-to-parent breadcrumb in the chat header so a user who
  // dropped into a worker thread (via `WorkerThreadRefCard` or the Tasks
  // bucket) can return to the conversation that spawned it
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

  // Thread list (left pane). Rendered through `TwoPanelLayout` below in page
  // mode; the embedded `variant="sidebar"` mode shows no thread list at all.
  const threadSidebar = (
    // Card background / rounded corners come from TwoPanelLayout's pane styling.
    <div className="h-full flex flex-col">
      {/* Thread search — flush full-width input, mirrors the settings search. */}
      <div className="relative border-b border-line-subtle">
        <span className="pointer-events-none absolute inset-y-0 left-3 flex items-center text-content-faint">
          <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M21 21l-4.35-4.35M11 19a8 8 0 100-16 8 8 0 000 16z"
            />
          </svg>
        </span>
        <input
          type="text"
          value={threadSearch}
          onChange={e => setThreadSearch(e.target.value)}
          onKeyDown={e => {
            if (e.key === 'Escape' && threadSearch) {
              e.preventDefault();
              setThreadSearch('');
            }
          }}
          placeholder={t('chat.searchThreads')}
          aria-label={t('chat.searchThreads')}
          data-testid="chat-thread-search-input"
          className="w-full border-0 bg-transparent py-2.5 pl-10 pr-10 text-sm text-content placeholder:text-stone-400 focus:outline-none focus:ring-0 dark:placeholder:text-neutral-500"
        />
        {threadSearch && (
          <button
            type="button"
            onClick={() => setThreadSearch('')}
            aria-label={t('settings.settingsSearch.clear')}
            data-testid="chat-thread-search-clear"
            className="absolute inset-y-0 right-2 flex items-center px-1 text-content-faint hover:text-content-secondary">
            <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M6 18L18 6M6 6l12 12"
              />
            </svg>
          </button>
        )}
      </div>
      {/* New conversation — a subtle, centered thread-style row (not a loud
          button), below the search and above the thread list. */}
      <button
        type="button"
        data-testid="new-thread-button"
        data-analytics-id="chat-sidebar-new-thread"
        onClick={() => void handleCreateNewThread()}
        title={t('chat.newThreadShortcut')}
        className="group w-full cursor-pointer border-b border-line-subtle/60 opacity-50 px-3 py-2 transition-colors hover:bg-surface-hover dark:border-line/60">
        <div className="flex items-center justify-center gap-1.5">
          <svg
            className="h-3.5 w-3.5 flex-shrink-0 text-content-muted"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 4v16m8-8H4" />
          </svg>
          <span className="truncate text-xs text-content-secondary">
            {t('chat.newConversation')}
          </span>
        </div>
      </button>
      <div className="flex-1 overflow-y-auto">
        {visibleThreads.length === 0 ? (
          <p className="px-4 py-6 text-xs text-content-faint text-center">{t('chat.noThreads')}</p>
        ) : (
          visibleThreads.map(thread => (
            <div
              key={thread.id}
              data-testid={`thread-row-${thread.id}`}
              data-analytics-id="chat-sidebar-thread-row"
              role="button"
              tabIndex={0}
              onClick={() => {
                dispatch(setSelectedThread(thread.id));
                void dispatch(loadThreadMessages(thread.id));
                if (shouldSyncChatRoute) {
                  navigate(chatThreadPath(thread.id));
                }
              }}
              onKeyDown={e => {
                if (e.target !== e.currentTarget) return;
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault();
                  dispatch(setSelectedThread(thread.id));
                  void dispatch(loadThreadMessages(thread.id));
                  if (shouldSyncChatRoute) {
                    navigate(chatThreadPath(thread.id));
                  }
                }
              }}
              className={`w-full text-left px-3 py-1.5 border-b border-line-subtle/60 dark:border-line/60 transition-colors group cursor-pointer ${
                selectedThreadId === thread.id
                  ? 'bg-primary-50 dark:bg-primary-900/30 border-l-2 border-l-primary-500'
                  : 'hover:bg-surface-hover'
              }`}>
              <div className="flex items-center justify-between">
                {editingThreadId === thread.id ? (
                  <input
                    ref={editTitleInputRef}
                    value={editTitleValue}
                    onClick={e => e.stopPropagation()}
                    onChange={e => setEditTitleValue(e.target.value)}
                    onKeyDown={e => {
                      e.stopPropagation();
                      // Ignore the Enter that confirms an IME composition
                      // candidate (CJK input) so it doesn't prematurely commit.
                      if (isImeCompositionKeyEvent(e)) return;
                      if (e.key === 'Enter') {
                        e.preventDefault();
                        handleCommitTitle(thread.id);
                      } else if (e.key === 'Escape') {
                        // Escape is an explicit cancel — suppress the commit the
                        // ensuing blur would otherwise fire.
                        ignoreNextTitleBlurRef.current = true;
                        setEditingThreadId(null);
                      }
                    }}
                    onBlur={() => {
                      if (ignoreNextTitleBlurRef.current) {
                        ignoreNextTitleBlurRef.current = false;
                        return;
                      }
                      handleCommitTitle(thread.id);
                    }}
                    aria-label={t('chat.editThreadTitle')}
                    data-testid={`thread-title-input-${thread.id}`}
                    className="h-5 min-w-0 flex-1 border-b border-primary-400 bg-transparent py-0 text-xs font-medium leading-none text-content-secondary outline-none"
                    autoFocus
                  />
                ) : (
                  <p
                    className={`text-xs truncate flex-1 ${
                      selectedThreadId === thread.id
                        ? 'font-medium text-primary-700 dark:text-primary-200'
                        : 'text-content-secondary'
                    }`}>
                    {resolveThreadDisplayTitle(thread.id)}
                  </p>
                )}
                <button
                  type="button"
                  data-analytics-id="chat-sidebar-edit-thread-title"
                  onClick={e => {
                    e.stopPropagation();
                    handleStartEditTitle(thread.id);
                  }}
                  aria-label={t('chat.editThreadTitle')}
                  title={t('chat.editThreadTitle')}
                  className="ml-2 p-1 rounded opacity-0 group-hover:opacity-100 hover:bg-surface-strong dark:bg-surface-muted dark:hover:bg-surface-muted text-content-faint hover:text-primary-500 transition-all flex-shrink-0">
                  <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M15.232 5.232l3.536 3.536m-2.036-5.036a2.5 2.5 0 113.536 3.536L6.5 21.036H3v-3.572L16.732 3.732z"
                    />
                  </svg>
                </button>
                <button
                  type="button"
                  data-analytics-id="chat-sidebar-delete-thread"
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
                        if (shouldSyncChatRoute && routeThreadId === thread.id) {
                          navigate('/chat', { replace: true });
                        }
                        void dispatch(deleteThread(thread.id));
                      },
                      onCancel: () => {},
                    });
                  }}
                  className="ml-2 p-1 rounded opacity-0 group-hover:opacity-100 hover:bg-surface-strong dark:bg-surface-muted dark:hover:bg-surface-muted text-content-faint hover:text-coral-500 transition-all flex-shrink-0"
                  title={t('chat.deleteThread')}>
                  <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
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
                    <span className="text-[10px] text-content-faint">
                      {formatRelativeTime(thread.lastMessageAt)}
                    </span>
                    {thread.messageCount > 0 && (
                      <span className="text-[10px] text-content-faint">
                        {thread.messageCount} msg{thread.messageCount !== 1 ? 's' : ''}
                      </span>
                    )}
                  </div> */}
            </div>
          ))
        )}
      </div>
    </div>
  );

  // Main chat area (right pane): header, message list, composer.
  const mainPanel = (
    <div
      className={
        isSidebar
          ? // Embedded variant keeps its own flush styling (no TwoPanelLayout).
            'flex-1 flex flex-col min-w-0 bg-surface border-l border-line overflow-hidden'
          : // Page variant: flush over the shell background. `relative` anchors
            // the absolutely-positioned floating composer.
            'relative flex-1 flex flex-col min-w-0'
      }>
      <div
        ref={messagesContainerRef}
        // Full-width scroll (scrollbar hugs the window edge); inner content is
        // centered and width-capped per branch below. `min-h-0` lets this
        // basis-0 flex child shrink to 0 so the composer footer can take the
        // space (and scroll) on short windows (#3785).
        className="flex-1 min-h-0 overflow-y-auto">
        {isLoadingMessages ? (
          <div className="mx-auto w-full max-w-[48.75rem] space-y-4 px-5 py-4">
            {Array.from({ length: 4 }).map((_, i) => (
              <div key={i} className={`flex ${i % 2 === 0 ? 'justify-start' : 'justify-end'}`}>
                <div
                  className={`h-12 rounded-2xl animate-pulse bg-surface-subtle ${
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
            <p className="text-sm text-content-faint mb-1">{t('chat.failedToLoadMessages')}</p>
            <p className="text-xs text-content-secondary mb-3 text-center">{messagesError}</p>
            <button
              type="button"
              data-analytics-id="chat-messages-reload"
              onClick={() => window.location.reload()}
              className="text-xs text-primary-400 hover:text-primary-300 transition-colors">
              {t('common.reload')}
            </button>
          </div>
        ) : hasVisibleMessages || hasTaskBoard ? (
          <div
            data-testid="chat-message-list"
            className={`mx-auto w-full max-w-[48.75rem] space-y-3 px-5 pt-4 ${
              isSidebar ? 'pb-4' : ''
            }`}
            // Page variant: reserve room for the absolutely-positioned floating
            // composer footer so its tail stays visible. Tracks the footer's
            // measured height (+16px gap) instead of a static `pb-32`, so the
            // queued-followups panel and other dynamic footer content never
            // overlap the last message (#4268).
            style={!isSidebar ? { paddingBottom: composerFooterHeight + 16 } : undefined}>
            {visibleMessages.map(msg => {
              const isAgentTextMode = msg.sender === 'agent' && agentMessageViewMode === 'text';
              // Parsed once per message: for current messages (extraMetadata
              // present, or agent messages) msg.content already has no markers,
              // so this is a no-op. For legacy persisted user messages with raw
              // [IMAGE:...]/[FILE:...] markers and no extraMetadata, this is
              // what keeps the marker text out of both the rendered bubble and
              // the copy-to-clipboard action.
              const parsedContent = parseMessageImages(msg.content ?? '');
              return (
                <Fragment key={msg.id}>
                  <div>
                    <div
                      className={`group/msg flex ${msg.sender === 'user' ? 'justify-end' : 'justify-start'}`}>
                      <div
                        className={`relative ${
                          isAgentTextMode ? 'w-full max-w-full' : 'w-fit max-w-[75%]'
                        }`}>
                        {msg.sender === 'agent' ? (
                          <div className="space-y-1">
                            <div className="relative space-y-1">
                              {agentMessageViewMode === 'text' ? (
                                <AgentMessageText content={msg.content} />
                              ) : (
                                splitAgentMessageIntoBubbles(msg.content).map(
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
                                )
                              )}
                              {/* Reaction affordance — the closed "+", the open picker,
                                and the resulting reaction chips all live here, tucked
                                onto the bubble's bottom-left corner so the control
                                never jumps to a separate row below the timestamp. */}
                              {latestVisibleMessage?.id === msg.id &&
                                (() => {
                                  const myReactions =
                                    (msg.extraMetadata?.myReactions as string[] | undefined) ?? [];
                                  const pickerOpen = reactionPickerMsgId === msg.id;
                                  return (
                                    <div className="absolute -bottom-2 left-3 z-10 flex items-center gap-1">
                                      {myReactions.map(emoji => (
                                        <button
                                          key={emoji}
                                          type="button"
                                          data-analytics-id="chat-message-reaction-remove"
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
                                          className="flex items-center rounded-full border border-primary-200 bg-primary-100 px-1.5 text-xs leading-[1.5] shadow-sm transition-colors hover:bg-primary-200 dark:border-primary-400/40 dark:bg-primary-500/25"
                                          title={t('chat.removeReaction').replace(
                                            '{emoji}',
                                            emoji
                                          )}>
                                          {emoji}
                                        </button>
                                      ))}
                                      {pickerOpen ? (
                                        <div className="flex items-center gap-0.5 rounded-full bg-surface px-1 py-0.5 shadow-sm ring-1 ring-stone-200 dark:ring-neutral-700">
                                          {['👍', '❤️', '😂', '🔥', '👀', '🎯'].map(emoji => (
                                            <button
                                              key={emoji}
                                              type="button"
                                              data-analytics-id="chat-message-reaction-pick"
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
                                              className="rounded px-0.5 text-sm transition-transform hover:scale-125"
                                              title={emoji}>
                                              {emoji}
                                            </button>
                                          ))}
                                          <button
                                            type="button"
                                            data-analytics-id="chat-message-reaction-close"
                                            onClick={() => setReactionPickerMsgId(null)}
                                            className="ml-0.5 px-0.5 text-xs text-content-secondary hover:text-content-faint dark:hover:text-content-faint">
                                            ✕
                                          </button>
                                        </div>
                                      ) : (
                                        <button
                                          type="button"
                                          data-analytics-id="chat-message-reaction-open"
                                          onClick={() => setReactionPickerMsgId(msg.id)}
                                          className="flex h-[18px] items-center rounded-full bg-surface px-1.5 text-xs leading-none text-content-muted opacity-0 shadow-sm ring-1 ring-stone-200 transition-opacity hover:bg-surface-hover hover:text-content-secondary group-hover/msg:opacity-100 dark:ring-neutral-700"
                                          title={t('chat.addReaction')}
                                          aria-label={t('chat.addReaction')}>
                                          +
                                        </button>
                                      )}
                                    </div>
                                  );
                                })()}
                            </div>
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
                              <p className="px-1 text-[10px] text-content-faint">
                                {formatRelativeTime(msg.createdAt)}
                              </p>
                            )}
                          </div>
                        ) : (
                          <div className="flex flex-col items-end gap-1">
                            {(() => {
                              const displayText = parsedContent.text;
                              const dataUris = Array.isArray(msg.extraMetadata?.attachmentDataUris)
                                ? (msg.extraMetadata.attachmentDataUris as string[])
                                : parsedContent.dataUris;
                              const hasImages = dataUris.length > 0;
                              // Document attachments carry no image data-URI (only
                              // images do); surface them as filename chips from the
                              // persisted attachmentKinds/attachmentNames metadata.
                              const kinds = Array.isArray(msg.extraMetadata?.attachmentKinds)
                                ? (msg.extraMetadata.attachmentKinds as string[])
                                : [];
                              const names = Array.isArray(msg.extraMetadata?.attachmentNames)
                                ? (msg.extraMetadata.attachmentNames as string[])
                                : [];
                              const fileNames = kinds
                                .map((k, i) => (k === 'file' ? names[i] : null))
                                .filter((n): n is string => Boolean(n));
                              const posters = Array.isArray(msg.extraMetadata?.attachmentPosters)
                                ? (msg.extraMetadata.attachmentPosters as (string | null)[])
                                : [];
                              const videoItems = kinds
                                .map((k, i) =>
                                  k === 'video'
                                    ? { name: names[i] ?? '', poster: posters[i] ?? null }
                                    : null
                                )
                                .filter((v): v is { name: string; poster: string | null } =>
                                  Boolean(v)
                                );
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
                                  {videoItems.length > 0 && (
                                    <div className="flex flex-wrap gap-1.5 justify-end">
                                      {videoItems.map((video, i) => (
                                        <div
                                          key={i}
                                          className="relative flex items-center gap-2 rounded-lg border border-line bg-surface-muted px-2.5 py-1.5 text-xs text-content-secondary max-w-[220px]">
                                          {video.poster ? (
                                            <div className="relative w-10 h-10 flex-shrink-0">
                                              <img
                                                src={video.poster}
                                                alt=""
                                                className="w-10 h-10 rounded object-cover"
                                              />
                                              <span className="absolute inset-0 flex items-center justify-center">
                                                <svg
                                                  className="w-4 h-4 text-white drop-shadow"
                                                  fill="currentColor"
                                                  viewBox="0 0 24 24">
                                                  <path d="M8 5v14l11-7z" />
                                                </svg>
                                              </span>
                                            </div>
                                          ) : (
                                            <svg
                                              className="w-4 h-4 flex-shrink-0 text-content-muted"
                                              fill="none"
                                              stroke="currentColor"
                                              viewBox="0 0 24 24">
                                              <path
                                                strokeLinecap="round"
                                                strokeLinejoin="round"
                                                strokeWidth={1.8}
                                                d="M15 10l4.553-2.276A1 1 0 0121 8.618v6.764a1 1 0 01-1.447.894L15 14M5 6h8a2 2 0 012 2v8a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2z"
                                              />
                                            </svg>
                                          )}
                                          <span className="truncate font-medium">{video.name}</span>
                                        </div>
                                      ))}
                                    </div>
                                  )}
                                  {fileNames.length > 0 && (
                                    <div className="flex flex-wrap gap-1.5 justify-end">
                                      {fileNames.map((name, i) => (
                                        <div
                                          key={i}
                                          className="flex items-center gap-2 rounded-lg border border-line bg-surface-muted px-2.5 py-1.5 text-xs text-content-secondary max-w-[220px]">
                                          <svg
                                            className="w-4 h-4 flex-shrink-0 text-content-muted"
                                            fill="none"
                                            stroke="currentColor"
                                            viewBox="0 0 24 24">
                                            <path
                                              strokeLinecap="round"
                                              strokeLinejoin="round"
                                              strokeWidth={1.8}
                                              d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z"
                                            />
                                            <path
                                              strokeLinecap="round"
                                              strokeLinejoin="round"
                                              strokeWidth={1.8}
                                              d="M14 2v6h6"
                                            />
                                          </svg>
                                          <span className="truncate font-medium">{name}</span>
                                        </div>
                                      ))}
                                    </div>
                                  )}
                                  {(displayText || showTime) && (
                                    <div className="rounded-2xl px-4 py-2.5 bg-primary-500 text-content-inverted rounded-br-md break-words overflow-hidden">
                                      {displayText && (
                                        <BubbleMarkdown content={displayText} tone="user" />
                                      )}
                                      {showTime && (
                                        <p
                                          className={`${displayText ? 'mt-1' : ''} text-[10px] text-white/60`}>
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
                          type="button"
                          data-analytics-id="chat-message-copy"
                          onClick={() => handleCopyMessage(msg.id, parsedContent.text)}
                          className={`absolute -top-1 ${
                            isAgentTextMode
                              ? 'right-0'
                              : msg.sender === 'user'
                                ? '-left-8'
                                : '-right-8'
                          } p-1 rounded-md opacity-0 group-hover/msg:opacity-100 hover:bg-surface-hover dark:bg-surface-muted dark:hover:bg-surface-muted text-content-faint hover:text-content-secondary transition-all`}
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
                      </div>
                    </div>
                  </div>
                  {msg.id === lastUserMessageId ? agentInsights : null}
                </Fragment>
              );
            })}
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
                  <div className="bg-surface-strong/80 dark:bg-surface-muted rounded-2xl rounded-bl-md px-4 py-3">
                    <div className="flex items-center gap-1">
                      <span className="w-1.5 h-1.5 rounded-full bg-surface-muted dark:bg-surface-muted/600 animate-bounce [animation-delay:0ms]" />
                      <span className="w-1.5 h-1.5 rounded-full bg-surface-muted dark:bg-surface-muted/600 animate-bounce [animation-delay:150ms]" />
                      <span className="w-1.5 h-1.5 rounded-full bg-surface-muted dark:bg-surface-muted/600 animate-bounce [animation-delay:300ms]" />
                    </div>
                  </div>
                </div>
              )}
            {/* Streaming assistant preview — compact trailing tail of the
                  in-flight response. Rendered as plain text (not Markdown) to
                  avoid jitter from partially-parsed fences. The final bubble
                  replaces this via addInferenceResponse on chat_done. */}
            {selectedStreamingAssistant &&
              (selectedStreamingAssistant.thinking.length > 0 ||
                (selectedStreamingAssistant.content.length > 0 &&
                  (selectedThreadToolTimeline.length === 0 || hideAgentInsights))) && (
                <div className="flex justify-start">
                  <div className="relative w-fit max-w-[75%]">
                    {selectedStreamingAssistant.thinking.length > 0 && (
                      <details className="mb-1.5 bg-surface-subtle rounded-lg px-3 py-1.5 text-xs text-content-secondary open:bg-stone-100 dark:bg-surface-muted dark:open:bg-neutral-800">
                        <summary className="cursor-pointer select-none flex items-center gap-1.5">
                          <span className="inline-block w-1.5 h-1.5 rounded-full bg-primary-400 animate-pulse" />
                          <span>{t('chat.thinking')}</span>
                        </summary>
                        <pre className="whitespace-pre-wrap break-words mt-1.5 font-sans text-[11px] text-content-muted">
                          {selectedStreamingAssistant.thinking.slice(-STREAMING_PREVIEW_CHARS)}
                        </pre>
                      </details>
                    )}
                    {selectedStreamingAssistant.content.length > 0 &&
                      (selectedThreadToolTimeline.length === 0 || hideAgentInsights) && (
                        <div className="rounded-2xl rounded-bl-md px-3 py-1.5 bg-surface-strong/80 dark:bg-surface-muted text-content">
                          <p className="text-xs text-content-secondary font-mono whitespace-pre-wrap break-words leading-snug">
                            {selectedStreamingAssistant.content.length >
                              STREAMING_PREVIEW_CHARS && (
                              <span className="text-content-faint">…</span>
                            )}
                            {selectedStreamingAssistant.content.slice(-STREAMING_PREVIEW_CHARS)}
                            <span className="inline-block w-1 h-3 ml-0.5 align-middle bg-primary-400 animate-pulse" />
                          </p>
                        </div>
                      )}
                  </div>
                </div>
              )}
            {/* Parallel (forked) branch streams — concurrent turns on this
                  thread, each its own labeled bubble so they don't collide with
                  the primary stream above. */}
            {selectedParallelStreams.map(
              branch =>
                (branch.content.length > 0 || branch.thinking.length > 0) && (
                  <div key={branch.requestId} className="flex justify-start">
                    <div className="relative w-fit max-w-[75%]">
                      <div className="mb-1 flex items-center gap-1.5 text-[10px] font-medium uppercase tracking-wide text-primary-500 dark:text-primary-400">
                        <span className="inline-block w-1.5 h-1.5 rounded-full bg-primary-400 animate-pulse" />
                        <span>{t('chat.parallelBranchLabel')}</span>
                      </div>
                      {branch.content.length > 0 && (
                        <div className="rounded-2xl rounded-bl-md px-3 py-1.5 bg-surface-strong/80 dark:bg-surface-muted text-content border-l-2 border-primary-400/60">
                          <p className="text-xs text-content-secondary font-mono whitespace-pre-wrap break-words leading-snug">
                            {branch.content.length > STREAMING_PREVIEW_CHARS && (
                              <span className="text-content-faint">…</span>
                            )}
                            {branch.content.slice(-STREAMING_PREVIEW_CHARS)}
                            <span className="inline-block w-1 h-3 ml-0.5 align-middle bg-primary-400 animate-pulse" />
                          </p>
                        </div>
                      )}
                    </div>
                  </div>
                )
            )}
            {/* Inference status indicator.
                  For the tool_use / subagent phases this line just restates the
                  active row already shown in the agentic-task-insights timeline,
                  so suppress it once that timeline is on screen — keep it only
                  for the `thinking` phase (which has no timeline row yet) or when
                  there is no timeline to fall back on. */}
            {selectedInferenceStatus &&
              (selectedInferenceStatus.phase === 'thinking' ||
                selectedThreadToolTimeline.length === 0) && (
                <div className="flex items-center gap-2 px-1 py-1.5 text-xs text-content-muted">
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
            {/* The "Agentic task insights" panel is rendered inline *above* the
                latest answer (right after the latest turn's user message) so
                processing reads before the result. This fallback only fires for
                the rare thread with no user message (e.g. a proactive-only
                thread) so the recorded steps are never unreachable. The cancel
                control + view-process-source opener now live in `agentInsights`
                and the floating footer respectively (upstream relocated the
                in-flow cancel button below the composer). */}
            {!lastUserMessageId && agentInsights}
            <div ref={messagesEndRef} />
          </div>
        ) : isNewWindow ? (
          <ChatNewWindowHero />
        ) : (
          <div className="flex-1 flex items-center justify-center h-full">
            <p className="text-sm text-content-secondary">{t('chat.noMessages')}</p>
          </div>
        )}
      </div>

      {/* Full-width fade so messages dissolve into the background (black/white
          per theme) behind the floating composer. Page variant only. */}
      {!isSidebar && (
        <div
          aria-hidden="true"
          className="pointer-events-none absolute inset-x-0 bottom-0 z-10 h-28 bg-gradient-to-t from-white via-white/90 to-transparent dark:from-black dark:via-black/90"
        />
      )}

      <div
        ref={composerFooterRef}
        data-walkthrough="home-cta"
        // Page variant: float at the bottom (absolute) over the fade; centered +
        // width-capped to match the messages. `z-20` keeps it above messages
        // that would otherwise paint over it while scrolling.
        //
        // Sidebar embed keeps the in-flow composer pinned at the bottom, but it
        // must stay reachable when the panel is too short to hold the whole
        // footer — it stacks the upsell/error banners + actionable error CTAs
        // (e.g. the voice "Setup" link) + the composer (#3785). Rather than a
        // percentage `max-height` (which does not reliably resolve inside a
        // stretched flex item in Chromium), let the footer SHRINK: dropping
        // `flex-shrink-0` and adding `min-h-0 overflow-y-auto` makes the flex
        // algorithm cap it to the available height (the basis-0 message list
        // gives up its space first) and scroll internally instead of being
        // clipped by the `overflow-hidden` mainPanel. On a tall window there is
        // free space, so the footer keeps its natural height (composer pinned).
        className={
          isSidebar
            ? 'mx-auto w-full max-w-[48.75rem] min-h-0 overflow-y-auto px-4 py-3'
            : 'absolute inset-x-0 bottom-0 z-20 mx-auto w-full max-w-[48.75rem] px-4 pb-4 pt-6'
        }>
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
            <div className="mb-3 p-3 rounded-xl bg-coral-50 border border-coral-200 flex flex-wrap items-center justify-between gap-3">
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
                <p className="text-xs text-coral-600">
                  {teamUsage.cycleBudgetUsd > 0
                    ? `${t('chat.weeklyLimitHit')}${teamUsage.cycleEndsAt ? ` ${t('chat.resets')} ${formatResetTime(teamUsage.cycleEndsAt)}.` : ''} ${t('chat.topUpToContinue')}`
                    : t('chat.budgetComplete')}
                </p>
              </div>
              <div className="flex flex-shrink-0 items-center gap-2">
                <button
                  type="button"
                  data-analytics-id="chat-budget-openrouter-free"
                  disabled={openRouterStatus === 'saving'}
                  onClick={() => {
                    void handleUseOpenRouterFree();
                  }}
                  className="px-3 py-1.5 rounded-lg border border-coral-300 bg-surface text-coral-700 hover:bg-coral-100 disabled:cursor-wait disabled:opacity-70 text-xs font-medium transition-colors">
                  {openRouterStatus === 'saving'
                    ? t('openrouterFree.saving')
                    : t('openrouterFree.cta')}
                </button>
                <button
                  type="button"
                  data-analytics-id="chat-budget-top-up"
                  onClick={() => {
                    void openUrl(BILLING_DASHBOARD_URL);
                  }}
                  className="px-3 py-1.5 rounded-lg bg-coral-500 hover:bg-coral-400 text-content-inverted text-xs font-medium transition-colors">
                  {t('chat.topUp')}
                </button>
              </div>
            </div>
          )}
          {openRouterStatus === 'error' && (
            <div className="mb-3 rounded-lg border border-coral-200 bg-coral-50 px-3 py-2 text-xs text-coral-700">
              {t('openrouterFree.error')}
            </div>
          )}

          {/* Cycle usage pill moved into ChatComposer toolbar */}
        </>

        {sendAdvisory && (
          <div className="flex items-center justify-between mb-2">
            <p className="text-xs text-amber-700" data-chat-send-advisory>
              {sendAdvisory}
            </p>
            <button
              type="button"
              data-analytics-id="chat-send-advisory-dismiss"
              onClick={() => setSendAdvisory(null)}
              className="text-xs text-content-muted hover:text-content-secondary dark:text-neutral-200 dark:hover:text-neutral-200 transition-colors ml-2">
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
              type="button"
              data-analytics-id="chat-attach-error-dismiss"
              onClick={() => setAttachError(null)}
              className="text-xs text-content-muted hover:text-content-secondary transition-colors ml-2">
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
                  type="button"
                  data-analytics-id="chat-send-error-setup"
                  onClick={() => {
                    setSendError(null);
                    // STT/TTS provider settings live on the Voice panel
                    // since PR 2; the legacy local-model route was for
                    // back when speech assets were lumped with Ollama.
                    navigate('/settings/voice', settingsNavState(location));
                  }}
                  className="text-xs text-primary-500 hover:text-primary-600 font-medium transition-colors">
                  {t('chat.setup')}
                </button>
              )}
              <button
                type="button"
                data-analytics-id="chat-send-error-dismiss"
                onClick={() => setSendError(null)}
                className="text-xs text-content-muted hover:text-content-secondary dark:text-neutral-200 dark:hover:text-neutral-200 transition-colors">
                {t('common.dismiss')}
              </button>
            </div>
          </div>
        )}

        {(() => {
          // Surface a parked ApprovalGate request for the shown thread just
          // above the composer, so it stays visible regardless of scroll.
          const approvalThreadId = selectedThreadId ?? firstActiveThreadId;
          const pendingApproval = approvalThreadId
            ? pendingApprovalByThread[approvalThreadId]
            : undefined;
          if (!pendingApproval || !approvalThreadId) return null;
          // `composio_connect` parks on the same gate but needs a Connect
          // button + OAuth poll rather than approve/deny (#3993).
          const isConnect = pendingApproval.toolName === 'composio_connect';
          return (
            <div className="mb-2">
              {isConnect ? (
                // Key by requestId so switching from one parked approval to
                // another remounts the card with fresh local state (phase,
                // field values, cancellation refs, poll timers) instead of
                // bleeding the previous request's state in (#4062, coderabbit).
                <IntegrationConnectCard
                  key={pendingApproval.requestId}
                  threadId={approvalThreadId}
                  approval={pendingApproval}
                />
              ) : (
                <ApprovalRequestCard
                  key={pendingApproval.requestId}
                  threadId={approvalThreadId}
                  approval={pendingApproval}
                />
              )}
            </div>
          );
        })()}

        {(() => {
          // Surface in-flight + failed artifact cards above the composer
          // (#2779). Mirrors the approval-card placement so the user sees
          // the spinner / error without scrolling. `ready` cards are
          // delegated to the header ChatFilesChip panel (#3024) so the
          // chat scroll area isn't permanently occupied — restored decks
          // are listable from the chip on demand.
          //
          // The failed-card Retry button re-dispatches the producing tool
          // via `ai_regenerate` (#3162): the core reloads the persisted
          // creation args and re-runs generation under the original
          // artifact id, so the card swaps back to a spinner in place and
          // then to ready/failed via the socket events.
          const artifactThreadId = selectedThreadId ?? firstActiveThreadId;
          const all = artifactThreadId ? (artifactsByThread[artifactThreadId] ?? []) : [];
          const live = all.filter(a => a.status !== 'ready');
          if (live.length === 0) return null;
          return (
            <div className="mb-2 flex flex-col gap-2">
              {live.map(artifact => (
                <ArtifactCard
                  key={artifact.artifactId}
                  artifact={artifact}
                  onRetry={
                    artifactThreadId
                      ? id => {
                          void aiRegenerate(id, artifactThreadId).catch(err => {
                            console.warn('[artifact] regenerate failed:', err);
                          });
                        }
                      : undefined
                  }
                />
              ))}
            </div>
          );
        })()}

        {/* Thread-scoped todo list the agent maintains as it works — read-only,
            pinned above the composer. Distinct from the Intelligence-tab kanban
            (global `user-tasks`). Renders nothing when the thread has no active
            cards. */}
        {/* Plan-mode review: the orchestrator parked the live turn on a
            thread-scoped plan (request_plan_review gate). Surface it for the
            user to Approve / Reject / send feedback on before anything executes;
            the card resolves the parked turn via plan_review_decide. */}
        {selectedThreadId && pendingPlanReview && (
          // Key by request id so a re-parked (revised) plan — or a thread switch —
          // remounts the card and resets its local decision/feedback state,
          // matching the ApprovalRequestCard pattern above.
          <PlanReviewCard
            key={pendingPlanReview.requestId}
            threadId={selectedThreadId}
            review={pendingPlanReview}
          />
        )}

        {selectedThreadId && (
          <ThreadTodoStrip
            board={selectedTaskBoard}
            disabled={!selectedThreadId}
            onViewSession={card => {
              if (!card.sessionThreadId) return;
              // Navigation only — do NOT mark the thread active. activeThreadId
              // tracks a true in-flight turn; forcing a completed session active
              // would wedge the composer.
              dispatch(setSelectedThread(card.sessionThreadId));
              void dispatch(loadThreadMessages(card.sessionThreadId));
              if (shouldSyncChatRoute) {
                navigate(chatThreadPath(card.sessionThreadId));
              }
            }}
          />
        )}

        {/* Cancel the in-flight turn for composer modes that don't render the
            text ChatComposer (mic-cloud + voice). The text composer carries its
            own in-box Stop button, so the footer control only appears for the
            non-text branches — otherwise voice/mic flows would have no way to
            stop a long-running generation. */}
        {isSending && rustChat && (composer === 'mic-cloud' || inputMode !== 'text') && (
          <div className="mb-2 flex justify-start px-1">
            <button
              type="button"
              data-analytics-id="chat-cancel-generation"
              onClick={handleStopGeneration}
              className="text-xs text-content-muted transition-colors hover:text-content-secondary">
              {t('common.cancel')}
            </button>
          </div>
        )}

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
          <>
            <ChatComposer
              inputValue={inputValue}
              setInputValue={setInputValue}
              onSend={handleComposerSend}
              onStopGeneration={rustChat ? handleStopGeneration : undefined}
              textInputRef={textInputRef}
              fileInputRef={fileInputRef}
              composerInteractionBlocked={composerInteractionBlocked}
              isSending={isSending}
              allowParallelSend={selectedThreadActive}
              attachments={attachments}
              onAttachFiles={handleAttachFiles}
              onRemoveAttachment={id => setAttachments(prev => prev.filter(a => a.id !== id))}
              attachError={attachError}
              onSwitchToMicCloud={() => setComposerOverride('mic-cloud')}
              handleInputKeyDown={handleInputKeyDown}
              inlineCompletionSuffix={inlineCompletionSuffix}
              isComposingTextRef={isComposingTextRef}
              maxAttachments={ATTACHMENT_MAX_IMAGES + ATTACHMENT_MAX_FILES}
              // Empty → no native `accept` filter (it greys valid files on
              // macOS/CEF). Type enforcement happens in handleAttachFiles via
              // validateAndReadFile, which honors modelSupportsVision.
              allowedMimeTypes={[]}
              attachmentsEnabled={CHAT_ATTACHMENTS_ENABLED}
              // Header stack above the input box (outside its blue focus ring):
              // queued follow-ups + the thread-goal editor (opened via the
              // footer "Set goal" trigger). Entries that render null are no-ops.
              headerSlots={[
                selectedThreadId && (queuedFollowupsByThread[selectedThreadId]?.length ?? 0) > 0 ? (
                  <QueuedFollowups
                    key="queued-followups"
                    items={queuedFollowupsByThread[selectedThreadId] ?? []}
                    onClear={() => void handleClearQueuedFollowups()}
                  />
                ) : null,
                <ThreadGoalEditorPanel key="thread-goal" ctl={threadGoal} />,
              ]}
            />
          </>
        ) : (
          <div className="flex items-center gap-2">
            <button
              type="button"
              data-analytics-id="chat-voice-switch-to-text"
              onClick={() => setInputMode('text')}
              disabled={isRecording || isTranscribing}
              className="w-10 h-10 flex items-center justify-center rounded-full border border-line bg-surface text-content-muted hover:text-content-secondary dark:text-neutral-200 dark:hover:text-neutral-200 hover:border-line-strong dark:hover:border-line-strong transition-colors disabled:opacity-40"
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
              data-analytics-id="chat-voice-record-toggle"
              onClick={() => {
                void handleVoiceRecordToggle();
              }}
              disabled={!rustChat || isSending || isTranscribing || !canUseMicrophoneApi}
              className={`px-4 py-2.5 rounded-xl text-sm font-medium transition-colors ${
                isRecording
                  ? 'bg-coral-500 hover:bg-coral-400 text-content-inverted'
                  : 'bg-primary-600 hover:bg-primary-500 text-content-inverted'
              } disabled:opacity-40 disabled:cursor-not-allowed`}>
              {isTranscribing
                ? t('chat.transcribing')
                : isRecording
                  ? t('chat.stopAndSend')
                  : t('chat.startTalking')}
            </button>
            <p className="text-xs text-content-faint truncate">
              {voiceStatus ??
                (isPlayingReply && replyMode === 'voice'
                  ? t('chat.playingVoiceReply')
                  : canUseMicrophoneApi
                    ? t('chat.voiceHint')
                    : t('chat.micUnavailable'))}
            </p>
          </div>
        )}
        {/* Worker-thread back-to-parent breadcrumb (page variant) — its own line. */}
        {!isSidebar && selectedThreadParent && (
          <button
            type="button"
            data-analytics-id="chat-header-back-to-parent-thread"
            onClick={() => {
              dispatch(setSelectedThread(selectedThreadParent.id));
              void dispatch(loadThreadMessages(selectedThreadParent.id));
              navigate(chatThreadPath(selectedThreadParent.id));
            }}
            className="mt-2 flex items-center gap-1 rounded px-1 text-[11px] font-medium text-primary-600 hover:text-primary-700 hover:underline focus:outline-none focus-visible:ring-2 focus-visible:ring-primary-300"
            data-testid="worker-thread-back-to-parent">
            <span aria-hidden="true">←</span>
            <span className="max-w-[16rem] truncate">
              {t('chat.backToThread').replace('{title}', selectedThreadParent.title)}
            </span>
          </button>
        )}

        {/* Thread title + inline rename moved to the sidebar thread list rows. */}

        {/* Model + token stats (left) and the quick/reasoning toggle + files
            chip (right) share one line. */}
        <div
          className="mt-2 flex items-center justify-between gap-2"
          data-walkthrough="chat-agent-panel">
          <div className="flex min-w-0 items-center gap-2">
            <ComposerTokenStats model={resolvedModel} threadId={selectedThreadId} />
            {/* Set/show the thread goal; click opens the editor above the composer. */}
            <ThreadGoalFooterTrigger ctl={threadGoal} />
          </div>
          {!isSidebar && (
            <div className="flex flex-shrink-0 items-center gap-2">
              <div
                className="flex h-7 items-center rounded-full border border-line bg-surface-subtle p-0.5"
                role="radiogroup"
                aria-label={t('chat.agentProfile.label')}>
                <button
                  type="button"
                  role="radio"
                  aria-checked={selectedAgentProfileId === 'default'}
                  data-analytics-id="chat-header-mode-quick"
                  onClick={() => void handleSelectAgentProfile('default')}
                  className={`rounded-full px-2.5 py-0.5 text-xs font-medium transition-all ${
                    selectedAgentProfileId === 'default'
                      ? 'bg-surface text-content shadow-sm'
                      : 'text-content-muted hover:text-content-secondary'
                  }`}>
                  {t('chat.agentProfile.quick')}
                </button>
                <button
                  type="button"
                  role="radio"
                  aria-checked={selectedAgentProfileId === 'reasoning'}
                  data-analytics-id="chat-header-mode-reasoning"
                  onClick={() => void handleSelectAgentProfile('reasoning')}
                  className={`rounded-full px-2.5 py-0.5 text-xs font-medium transition-all ${
                    selectedAgentProfileId === 'reasoning'
                      ? 'bg-surface text-content shadow-sm'
                      : 'text-content-muted hover:text-content-secondary'
                  }`}>
                  {t('chat.agentProfile.reasoning')}
                </button>
              </div>
              {/* Super context is read at thread construction, so it only
                  affects NEW threads. Hide the toggle once the thread has ANY
                  activity — use the raw `messages` (not `hasVisibleMessages`,
                  which ignores hidden transcript entries) so an already-started
                  thread never looks "fresh" here. */}
              {messages.length === 0 && <SuperContextToggle />}
              {selectedThreadId && (
                <button
                  type="button"
                  data-testid="background-processes-toggle"
                  data-analytics-id="chat-header-background-processes"
                  onClick={() => setShowBackgroundProcesses(true)}
                  aria-label={t('conversations.backgroundTasks.title')}
                  title={
                    backgroundProcesses.length > 0
                      ? t('conversations.backgroundTasks.titleWithCount').replace(
                          '{count}',
                          String(backgroundProcesses.length)
                        )
                      : t('conversations.backgroundTasks.title')
                  }
                  className="relative flex h-7 w-7 items-center justify-center rounded-lg text-content-muted transition-colors hover:bg-surface-hover hover:text-content-secondary">
                  <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M19.428 15.428a2 2 0 00-1.022-.547l-2.387-.477a6 6 0 00-3.86.517l-.318.158a6 6 0 01-3.86.517L6.05 15.21a2 2 0 00-1.806.547M8 4h8l-1 1v5.172a2 2 0 00.586 1.414l5 5c1.26 1.26.367 3.414-1.415 3.414H4.828c-1.782 0-2.674-2.154-1.414-3.414l5-5A2 2 0 009 10.172V5L8 4z"
                    />
                  </svg>
                  {runningBackgroundCount > 0 ? (
                    <span className="absolute -right-0.5 -top-0.5 flex h-3.5 min-w-3.5 items-center justify-center rounded-full bg-amber-500 px-0.5 text-[9px] font-semibold leading-none text-content-inverted">
                      {runningBackgroundCount}
                    </span>
                  ) : memorySyncActive ? (
                    <span
                      data-testid="background-activity-dot"
                      className="absolute -right-0.5 -top-0.5 h-2 w-2 animate-pulse rounded-full bg-amber-500"
                    />
                  ) : null}
                </button>
              )}
              {(selectedThreadId ?? firstActiveThreadId) && (
                <ChatFilesChip threadId={(selectedThreadId ?? firstActiveThreadId) as string} />
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );

  return (
    <div
      className={
        isSidebar
          ? 'h-full relative z-10 flex overflow-hidden'
          : 'h-full relative z-10 flex justify-center overflow-hidden bg-surface/70 dark:bg-black/40'
      }>
      {isSidebar ? (
        <>
          {projectThreadList && (
            <SidebarContent>
              <div className="order-1 flex h-full min-h-0 flex-col overflow-hidden">
                {threadSidebar}
              </div>
            </SidebarContent>
          )}
          {mainPanel}
        </>
      ) : (
        // The thread list always lives in the root app sidebar's dynamic region
        // (order-1 so any app rail projected by the parent sits above it). The
        // chat pane keeps a comfortable, centered reading width.
        <>
          <SidebarContent>
            <div className="order-1 flex h-full min-h-0 flex-col overflow-hidden">
              {threadSidebar}
            </div>
          </SidebarContent>
          <div className="flex h-full w-full">{mainPanel}</div>
        </>
      )}
      <ConfirmationModal
        modal={deleteModal}
        onClose={() => setDeleteModal(prev => ({ ...prev, isOpen: false }))}
      />
      <BackgroundProcessesPanel
        open={showBackgroundProcesses}
        processes={backgroundProcesses}
        onClose={() => setShowBackgroundProcesses(false)}
        onOpenProcess={taskId => {
          setShowBackgroundProcesses(false);
          setOpenSubagentTaskId(taskId);
        }}
      />
      <SubagentDrawer
        key={openSubagentTaskId ?? 'none'}
        subagent={openSubagentEntry?.subagent ?? null}
        status={openSubagentEntry?.status}
        onCancel={
          openSubagentEntry?.subagent && selectedThreadId
            ? async () => {
                const taskId = openSubagentEntry.subagent!.taskId;
                const result = await subagentApi.cancel(taskId);
                // Only flip the row when something was actually aborted — a
                // cancelled=false result means the run already finished/unknown,
                // and overwriting its real terminal state would hide it. No
                // terminal socket event arrives for an aborted run, so the
                // optimistic mark is what surfaces the cancellation (the notice
                // itself reaches chat via the idle-gated delivery path).
                if (result.cancelled) {
                  dispatch(
                    markSubagentCancelled({ threadId: selectedThreadId, taskId: result.taskId })
                  );
                }
              }
            : undefined
        }
        onClose={() => setOpenSubagentTaskId(null)}
      />
      <AgentProcessSourcePanel
        open={showProcessSource}
        entries={selectedThreadToolTimeline}
        transcript={selectedThreadProcessing}
        scopedEntry={scopedDetailEntry}
        onClose={() => {
          setShowProcessSource(false);
          setScopedDetailEntryId(null);
        }}
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
