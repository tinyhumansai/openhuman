import debugFactory from 'debug';
import { type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useLocation, useNavigate, useParams } from 'react-router-dom';

import { type ChatSendError, chatSendError } from '../../chat/chatSendError';
import { checkPromptInjection, promptGuardMessage } from '../../chat/promptInjectionGuard';
import { trackAnalyticsEvent } from '../../components/analytics';
import { AgentStatus } from '../../components/assistant-ui/elements/agent-status';
import { TodoList } from '../../components/assistant-ui/elements/todo-list';
import ChatFilesChip from '../../components/chat/ChatFilesChip';
import WorkflowProposalCard from '../../components/chat/WorkflowProposalCard';
import { ConfirmationModal } from '../../components/intelligence/ConfirmationModal';
import { SidebarContent } from '../../components/layout/shell/SidebarSlot';
import { ArtifactCardAdapter } from '../../features/conversations/aui/ArtifactCardAdapter';
import { ContextUsage } from '../../features/conversations/aui/ContextUsage';
import { PlanReviewCardCore } from '../../features/conversations/aui/PlanReviewPart';
import { toAuiTodoItems } from '../../features/conversations/aui/TodoListPart';
import { useRunMode } from '../../features/conversations/aui/useRunMode';
import {
  formatTokens,
  useLoadThreadGoal,
  useThreadGoal,
} from '../../features/conversations/aui/useThreadGoal';
import {
  useLoadThreadTodos,
  useThreadTodos,
} from '../../features/conversations/aui/useThreadTodos';
import { AssistantUiChat } from '../../features/conversations/components/AssistantUiChat';
import { TranscriptOverlays } from '../../features/conversations/components/aui/TranscriptOverlays';
import {
  evaluateComposerSend,
  getComposerBlockedSendFeedback,
  handleComposerSlashCommand,
} from '../../features/conversations/composerSendDecision';
import { selectBackgroundProcesses } from '../../features/conversations/selectors/backgroundProcesses';
import {
  GENERAL_TAB_VALUE,
  isThreadVisibleInTab,
} from '../../features/conversations/utils/threadFilter';
import {
  ChatMascotDock,
  useChatMascotOptional,
  useChatMascotSendBinding,
} from '../../features/human/chatMascot';
import MicComposer from '../../features/human/MicComposer';
import { useFlowApprovalRequests } from '../../hooks/useFlowApprovalRequests';
import { useUnroutedApprovals } from '../../hooks/useUnroutedApprovals';
import { useUsageState } from '../../hooks/useUsageState';
import {
  type Attachment,
  ATTACHMENT_MAX_FILES,
  ATTACHMENT_MAX_IMAGES,
  buildMessageWithAttachments,
  imageMarkerCost,
  parseMessageImages,
  validateAndReadFile,
} from '../../lib/attachments';
import { useRegisterAction } from '../../lib/commands/useRegisterAction';
import { useT } from '../../lib/i18n/I18nContext';
import { decideApproval } from '../../services/api/approvalApi';
import { threadApi } from '../../services/api/threadApi';
import { fetchThreadTokenUsage } from '../../services/api/threadUsageApi';
import { aiRegenerate, chatCancel, chatSend, useRustChat } from '../../services/chatService';
import { callCoreRpc } from '../../services/coreRpcClient';
import {
  beginInferenceTurn,
  clearRuntimeForThread,
  clearThreadSendPending,
  fetchAndHydrateTurnState,
  hydrateThreadUsage,
  markThreadSendPending,
  type ProcessingTranscriptItem,
  setToolTimelineForThread,
  type ToolTimelineEntry,
} from '../../store/chatRuntimeSlice';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
import { pendingFollowupAdded } from '../../store/queueSlice';
import { selectSocketStatus } from '../../store/socketSelectors';
import {
  addMessageLocal,
  clearCreateThreadError,
  clearThreadInferenceActive,
  createNewThread,
  deleteThread,
  loadThreadMessages,
  loadThreads,
  markThreadInferenceActive,
  setSelectedThread,
  THREAD_NOT_FOUND_MESSAGE,
  updateThreadTitle,
} from '../../store/threadSlice';
import type { ConfirmationModal as ConfirmationModalType } from '../../types/intelligence';
import type { ThreadMessage } from '../../types/thread';
import { chatThreadPath } from '../../utils/chatRoutes';
import { CHAT_ATTACHMENTS_ENABLED } from '../../utils/config';
import {
  openhumanGetClientConfig,
  openhumanUpdateRuntimeSettings,
} from '../../utils/tauriCommands/config';
import { ApprovalCardAdapter } from './aui/ApprovalCardAdapter';
import { ComposerMessageQueue } from './aui/ComposerMessageQueue';
import {
  type ReasoningEffortChoice,
  ReasoningEffortPicker,
  toReasoningEffortChoice,
} from './aui/ReasoningEffortPicker';
import { useChatSurfaceRegistration } from './hooks/useChatSurfaceRegistration';
import { ThreadList } from './threadList/ThreadList';

const CHAT_MODEL_HINT = 'hint:chat';
const debug = debugFactory('conversations');
// How long a turn may go without any inference signal (status, stream delta,
// tool activity, or the core's 20s `inference_heartbeat`) before the chat
// warns that it has gone quiet. A warning only — see `handleSilence`.
const SILENCE_WARNING_MS = 120_000;

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
   * Voice-chat control rendered in the `mic-cloud` composer slot, above the mic
   * button. Passed in as a node rather than imported here so this component
   * keeps no dependency on the realtime voice stack (and the ElevenLabs SDK
   * stays out of every consumer's module graph). Ignored outside `mic-cloud`.
   */
  voiceChatControl?: ReactNode;
  /**
   * Whether the `mic-cloud` slot renders the push-to-talk mic composer. Default
   * `true` — set `false` alongside {@link ConversationsProps.voiceChatControl}
   * to replace tap-and-speak with the realtime control rather than stack them.
   */
  showMicComposer?: boolean;
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

// Stable empty live tool-timeline / processing-transcript for the selected
// thread. A fresh `[]` here took a new identity every render, invalidating the
// `backgroundProcesses` memo below on each pass and adding avoidable re-render
// churn to the chat's hot path (#5162).
const EMPTY_TOOL_TIMELINE: ToolTimelineEntry[] = [];
const EMPTY_PROCESSING: ProcessingTranscriptItem[] = [];

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

/**
 * What the error strip above the composer renders: this turn's send failure if
 * there is one, otherwise a thread-create failure recorded by `threadSlice`.
 *
 * A create that blew the 30 s RPC budget used to have no surface at all — the
 * shell's "New chat" / Home actions caught the rejection and dropped it, so the
 * button just did nothing, and a call site that forgot to catch turned the same
 * failure into `UnhandledRejection: … threads_create_new timed out after
 * 30000ms` (#5156). Routing the slice-recorded failure through the existing
 * banner gives every create path one visible outcome. Exported so the precedence
 * rule is unit-testable without mounting the page.
 */
export function deriveChatErrorBanner(
  sendError: ChatSendError | null,
  createThreadError: string | null,
  createThreadFailedMessage: string
): ChatSendError | null {
  if (sendError) return sendError;
  if (createThreadError) {
    return chatSendError('create_thread_failed', createThreadFailedMessage);
  }
  return null;
}

const Conversations = ({
  variant = 'page',
  composer: composerProp = 'text',
  voiceChatControl = null,
  showMicComposer = true,
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
  const { threads, selectedThreadId, messages } = useAppSelector(state => state.thread);
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

  const [inputValue, setInputValue] = useState('');
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  // What ingest counts its budget against. Tracks state on every render (so a
  // removal or a send's clear is picked up) and is written synchronously as each
  // file is admitted, which is what keeps two overlapping ingests honest.
  const attachmentsRef = useRef(attachments);
  attachmentsRef.current = attachments;
  // Tail of the ingest queue; see `handleAttachFiles`.
  const ingestQueueRef = useRef<Promise<void>>(Promise.resolve());
  // Disclosure state for the transcript-local overlays (background
  // processes, the Agent Process Source panel).
  const [showBackgroundProcesses, setShowBackgroundProcesses] = useState(false);
  const [showProcessSource, setShowProcessSource] = useState(false);
  // The Agent Process Source panel (the whole-run view, and the visited-source
  // list, which exists nowhere else) is reached from the command palette:
  // assistant-ui renders tool calls as inline cards with no run-level footer to
  // hang a link on. Both composers (text and `mic-cloud`) share the one
  // assistant-ui panel that hosts `TranscriptOverlays`, so it is always
  // reachable while a thread is selected.
  useRegisterAction({
    id: 'chat.agentProcessSource',
    label: 'Open agent process source',
    labelKey: 'conversations.agentTaskInsights.viewProcessSource',
    group: 'Chat',
    handler: () => setShowProcessSource(true),
    enabled: () => {
      if (selectedThreadId === null) return false;
      return (
        (toolTimelineByThread[selectedThreadId]?.length ?? 0) > 0 ||
        (processingByThread[selectedThreadId]?.length ?? 0) > 0 ||
        Object.values(turnTimelinesByThread[selectedThreadId] ?? {}).some(
          entries => entries.length > 0
        ) ||
        Object.values(turnTranscriptsByThread[selectedThreadId] ?? {}).some(
          transcript => transcript.length > 0
        )
      );
    },
    keywords: ['agent', 'process', 'source', 'timeline', 'run'],
  });
  // Thread-list filtering is fixed to the General bucket — the in-sidebar
  // General/Tasks chips were removed. Task/worker threads have a dedicated
  // surface (Tasks board).
  const selectedLabel = GENERAL_TAB_VALUE;
  const [sendError, setSendError] = useState<ChatSendError | null>(null);
  // Recorded by the slice for *every* create path (#5156) — including the shell's
  // "New chat" button and the home-nav shortcut, which have no UI of their own —
  // so a failed create always has somewhere to show up.
  // Optional-chain + default, same as `activeThreadIds` below: narrow test
  // stores predate this field.
  const createThreadError = useAppSelector(state => state.thread.createThreadError ?? null);
  const [attachError, setAttachError] = useState<ChatSendError | null>(null);
  const [sendAdvisory, setSendAdvisory] = useState<string | null>(null);
  // Refs mirroring error/advisory state for effects that read them without
  // depending on them, preventing the classic "effect→setState→re-fire" cascade
  // that contributes to "Maximum update depth exceeded" (TAURI-REACT-2G).
  const sendErrorRef = useRef(sendError);
  sendErrorRef.current = sendError;
  const createThreadErrorRef = useRef(createThreadError);
  createThreadErrorRef.current = createThreadError;
  const displayedSendError = deriveChatErrorBanner(
    sendError,
    createThreadError,
    t('chat.createThreadFailed')
  );
  const sendAdvisoryRef = useRef(sendAdvisory);
  sendAdvisoryRef.current = sendAdvisory;
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
  // Optional chain because narrow test stores (e.g. Conversations.test
  // bootstraps without the locale slice) shouldn't crash here. `'en'`
  // matches the no-locale-directive branch in the core, so legacy
  // behaviour stays intact.
  const uiLocale = useAppSelector(state => state.locale?.current ?? 'en');
  const toolTimelineByThread = useAppSelector(state => state.chatRuntime.toolTimelineByThread);
  const turnTimelinesByThread = useAppSelector(state => state.chatRuntime.turnTimelinesByThread);
  const processingByThread = useAppSelector(state => state.chatRuntime.processingByThread);
  const turnTranscriptsByThread = useAppSelector(
    state => state.chatRuntime.turnTranscriptsByThread
  );
  const inferenceStatusByThread = useAppSelector(
    state => state.chatRuntime.inferenceStatusByThread
  );
  const artifactsByThread = useAppSelector(state => state.chatRuntime.artifactsByThread);
  // Flow-approval surface (chat): a paused tinyflows run's gate, pushed via
  // the `flow_approval_request` socket event. Not thread-scoped — the
  // payload carries no `thread_id` — so it's tracked independently of the
  // selected thread and surfaced regardless of which one is open.
  const { requests: flowApprovalRequests, dismiss: dismissFlowApprovalRequest } =
    useFlowApprovalRequests();
  // Approvals no other surface will show: a background trigger run has no chat
  // thread and no flow context, so the gate parks it, nothing asks the user,
  // and it TTL-denies after 600s (#6406; general form #5746). Polled from the
  // durable `approval_list_pending` queue rather than a socket event, because
  // the whole point is that it can be raised while nobody is watching.
  const {
    approvals: unroutedApprovals,
    decidingId: unroutedDecidingId,
    error: unroutedApprovalError,
    decide: decideUnroutedApproval,
  } = useUnroutedApprovals();
  const pendingPlanReviewByThread = useAppSelector(
    state => state.chatRuntime.pendingPlanReviewByThread
  );
  const pendingWorkflowProposalsByThread = useAppSelector(
    state => state.chatRuntime.pendingWorkflowProposalsByThread
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
  const inferenceTurnLifecycleByThread = useAppSelector(
    state => state.chatRuntime.inferenceTurnLifecycleByThread
  );
  const rustChat = useRustChat();
  // Inline thread-title rename in the sidebar thread list — keyed by the
  // thread id being edited (null = none) so any row can rename in place.
  const [editingThreadId, setEditingThreadId] = useState<string | null>(null);
  const [editTitleValue, setEditTitleValue] = useState('');
  const editTitleInputRef = useRef<HTMLInputElement>(null);
  const ignoreNextTitleBlurRef = useRef(false);

  const {
    isAtLimit,
    // #3767: gate on the tier for the selected chat mode — Quick runs on the
    // `chat` tier, Reasoning on the `reasoning` tier — so the credits prompt
    // reflects the mode the user actually picked.
    //
    // Only `isAtLimit` is read here now: the near-limit and spent-budget
    // banners this file rendered are notices in `NoticeCenter`, which reads
    // the same hook once for the whole app.
  } = useUsageState('chat');
  const [deleteModal, setDeleteModal] = useState<ConfirmationModalType>({
    isOpen: false,
    title: '',
    message: '',
    onConfirm: () => {},
    onCancel: () => {},
  });
  const [resolvedModel, setResolvedModel] = useState<string | null>(null);
  // The composer's picker choice. It overrides the model route for subsequent
  // sends immediately, and is also written to the core's `default_model` so
  // the pick survives an app restart and is what every managed turn runs on
  // (the same field Settings → Routing → "Default model" edits). `null` clears
  // the pin back to the managed default.
  const [composerModelOverride, setComposerModelOverride] = useState<string | null>(null);
  // `undefined` means no explicit picker selection, so usage-reported context
  // remains authoritative. `null` means the selected model did not report a
  // window, and the meter deliberately shows an unknown limit.
  const [composerModelContextWindow, setComposerModelContextWindow] = useState<
    number | null | undefined
  >(undefined);
  // The composer's thinking level. Sent with every turn (`reasoning_effort`)
  // so it applies immediately, and written to the core's
  // `runtime.reasoning_effort` so it survives a restart and is the default for
  // turns the composer does not start — the same split as the model pick.
  const [composerReasoningEffort, setComposerReasoningEffort] =
    useState<ReasoningEffortChoice>('default');
  useEffect(() => {
    let cancelled = false;
    void openhumanGetClientConfig()
      .then(res => {
        if (!cancelled) {
          setComposerReasoningEffort(toReasoningEffortChoice(res.result?.reasoning_effort));
        }
      })
      .catch((err: unknown) => {
        console.debug('[chat][composer-reasoning] client config unavailable', {
          message: err instanceof Error ? err.message : String(err),
        });
      });
    return () => {
      cancelled = true;
    };
  }, []);
  const applyComposerReasoningEffort = useCallback((value: ReasoningEffortChoice) => {
    setComposerReasoningEffort(value);
    void openhumanUpdateRuntimeSettings({ reasoning_effort: value === 'default' ? '' : value })
      .then(() => {
        console.debug('[chat][composer-reasoning] persisted reasoning_effort', { effort: value });
      })
      .catch((err: unknown) => {
        // The per-send value still applies; only persistence failed.
        console.warn('[chat][composer-reasoning] failed to persist reasoning_effort', {
          message: err instanceof Error ? err.message : String(err),
        });
      });
  }, []);
  const applyComposerModel = useCallback((value: string | null, contextWindow?: number | null) => {
    setComposerModelOverride(value);
    setComposerModelContextWindow(contextWindow ?? null);
    void callCoreRpc({
      method: 'openhuman.inference_update_model_settings',
      params: { default_model: value ?? '' },
    })
      .then(() => {
        console.debug('[chat][composer-model] persisted default_model', { pinned: value !== null });
      })
      .catch((err: unknown) => {
        // The in-session override still applies; only persistence failed.
        console.warn('[chat][composer-model] failed to persist default_model', {
          message: err instanceof Error ? err.message : String(err),
        });
      });
  }, []);

  // Whether the resolved model accepts image input.
  // Managed tiers do; custom/BYOK models only when the user flagged them. Gates
  // the composer's image-attachment affordance (docs flow regardless). Resolved
  // against the non-attachment hint so the affordance is stable as you attach.
  const [modelSupportsVision, setModelSupportsVision] = useState(false);
  // Whether a vision-capable delegate (the `vision` sub-agent) is reachable.
  // When it is, an image may be attached and routed to that sub-agent even if
  // the active orchestrator model is non-vision — the orchestrator sees a text
  // placeholder and delegates the image to the vision sub-agent. Resolved from
  // the `vision` workload route (the managed default on the managed backend, or the BYOK
  // model routed to the Vision workload).
  const [visionDelegateAvailable, setVisionDelegateAvailable] = useState(false);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        // Resolve the standard chat model so `modelSupportsVision` reflects the
        // normal agent path, AND the vision workload so we know whether a
        // vision sub-agent can take the image. Documents are text-extracted so
        // any model handles them.
        const hint = composerModelOverride ?? CHAT_MODEL_HINT;
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
  }, [composerModelOverride]);

  // One-shot guard for the Stop/ESC partial-preservation path (#4862): request
  // ids whose partial reply has already been persisted, so a repeated Stop/ESC
  // fired before the `cancelled` event clears the live stream can't append the
  // same partial twice.
  // Threads with an in-flight send, guarding against double-submit to the SAME
  // thread. Per-thread (a Set) so a send to thread B isn't blocked by an
  // in-flight send to thread A.
  const pendingSendsRef = useRef<Set<string>>(new Set());
  // Per-thread silence timers. Each in-flight turn gets its own 120s watchdog
  // keyed by thread id, so concurrent turns on different threads don't share
  // (and clobber) a single timeout.
  const sendingTimeoutsRef = useRef<Map<string, ReturnType<typeof setTimeout>>>(new Map());
  // Threads whose turn has gone quiet past the silence window while the core
  // still reports it running (or cannot say). Drives a warning only — the turn
  // is never torn down client-side; the Stop button is how the user ends it.
  const [stalledThreadIds, setStalledThreadIds] = useState<ReadonlySet<string>>(() => new Set());
  const markThreadStalled = useCallback((threadId: string) => {
    setStalledThreadIds(prev => {
      if (prev.has(threadId)) return prev;
      const next = new Set(prev);
      next.add(threadId);
      return next;
    });
  }, []);
  const clearThreadStalled = useCallback((threadId: string) => {
    setStalledThreadIds(prev => {
      if (!prev.has(threadId)) return prev;
      const next = new Set(prev);
      next.delete(threadId);
      return next;
    });
  }, []);
  // Live for as long as this instance is: flipped in the unmount cleanup so an
  // async continuation cannot schedule a watchdog onto a torn-down page.
  const isMountedRef = useRef(true);
  // Ref so the mount-time dictation event handler can call the latest send fn.
  const handleSendMessageRef = useRef<((text?: string) => Promise<void>) | null>(null);
  // Refs the assistant-ui chat-surface registration binds through. Both target
  // functions are re-created every render; the registration must NOT be, or the
  // registry slot would be rewritten on every keystroke and could be dropped
  // mid-turn. So the effect below depends only on the thread id and reads the
  // latest implementation out of these refs at call time.
  const handleComposerSendRef = useRef<((text?: string) => Promise<void>) | null>(null);
  const handleStopGenerationRef = useRef<(() => void) | null>(null);
  // Typed `/plan` / `/build` (see `handleSlashCommand`) flip the same run mode
  // the composer toggle and the `/` popover do.
  const { setMode: setRunMode } = useRunMode(selectedThreadId);
  // Per-thread "turn signature": the last-seen tuple of progress-slice
  // references [inferenceStatus, streamingAssistant, toolTimeline]
  // for each thread that owns a live silence timer. Redux Toolkit (immer)
  // only produces new references for the thread whose slice actually changed,
  // so comparing references lets the rearm effect (a) detect a turn completing
  // (status defined → undefined) and (b) rearm a thread's timer ONLY when that
  // thread's own state changed — unrelated threads' activity must not keep a
  // foreground turn's timer alive.
  const turnSignatureByThreadRef = useRef<Map<string, readonly unknown[]>>(new Map());

  const handleCreateNewThread = async () => {
    try {
      const thread = await dispatch(createNewThread()).unwrap();
      dispatch(setSelectedThread(thread.id));
      void dispatch(loadThreadMessages(thread.id));
      if (shouldSyncChatRoute) {
        debug('[chat][route] created thread thread=%s navigate=true', thread.id);
        navigate(chatThreadPath(thread.id));
      } else {
        debug('[chat][route] created thread thread=%s navigate=false', thread.id);
      }
    } catch (error) {
      debug('[chat] create thread failed: %O', error);
      setSendError(chatSendError('create_thread_failed', t('chat.createThreadFailed')));
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
        // session (task / worker / meeting) is filtered out of
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
    }
  }, [selectedThreadId, dispatch]);

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

      // A dictated draft needs a text surface where the user can inspect and
      // send it. The mic-first composer has neither, so hand it back first.
      setComposerOverride('text');
      setInputValue(prev => {
        const base = prev.trim();
        if (!base) return text;
        return `${base}${base.endsWith(' ') ? '' : ' '}${text}`;
      });
    };

    window.addEventListener('dictation://insert-text', onDictationInsert as EventListener);
    return () =>
      window.removeEventListener('dictation://insert-text', onDictationInsert as EventListener);
  }, []);

  useEffect(() => {
    if (sendErrorRef.current && inputValue.length > 0) {
      setSendError(null);
    }
    // The store-recorded create failure (#5156) dismisses on the same signal:
    // the user is composing, so they have seen it.
    if (createThreadErrorRef.current && inputValue.length > 0) {
      dispatch(clearCreateThreadError());
    }
    if (sendAdvisoryRef.current && inputValue.length > 0) {
      setSendAdvisory(null);
    }
    // Reads sendError/sendAdvisory through refs to avoid re-firing when they
    // are cleared — which would cascade into extra render cycles and contribute
    // to "Maximum update depth exceeded" (TAURI-REACT-2G).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [inputValue]);

  const clearSilenceTimer = useCallback((threadId: string) => {
    const existing = sendingTimeoutsRef.current.get(threadId);
    if (existing) {
      clearTimeout(existing);
      sendingTimeoutsRef.current.delete(threadId);
    }
  }, []);

  const armSilenceTimer = (threadId: string) => {
    clearSilenceTimer(threadId);
    // Never schedule onto a torn-down instance. `handleSendMessage` awaits
    // `addMessageLocal` before arming, so an unmount landing inside that await
    // runs the cleanup below — which finds nothing — and the continuation then
    // schedules a timer no cleanup will ever reach. The send itself is
    // unaffected; only the watchdog is skipped, which is correct: a page that
    // is gone cannot supervise a turn.
    if (!isMountedRef.current) {
      debug(`armSilenceTimer: instance unmounted — not scheduling for ${threadId}`);
      return;
    }
    const timeout = setTimeout(() => {
      sendingTimeoutsRef.current.delete(threadId);
      void handleSilence(threadId);
    }, SILENCE_WARNING_MS);
    sendingTimeoutsRef.current.set(threadId, timeout);
  };

  // The silence window elapsed with no inference signal for `threadId`.
  //
  // Silence is not failure. A reasoning model can think for minutes without
  // streaming anything, and a tool (a shell loop, a slow fetch) can run past
  // two minutes, so the watchdog never cancels or clears a turn on its own —
  // doing so discarded live, progressing work. It asks the core instead:
  //
  // - core says the turn already ended (`completed` / `interrupted`): the
  //   terminal event was lost (e.g. across a reconnect), so settle the local
  //   state and reload the thread — nothing is running to discard.
  // - otherwise (still running, no snapshot, or the lookup failed): keep the
  //   turn, show a warning, and keep watching. Any later signal clears the
  //   warning through the rearm effect; Stop remains the way to cancel.
  const handleSilence = async (threadId: string) => {
    debug(`silence: no inference signal for ${SILENCE_WARNING_MS}ms — checking core (${threadId})`);
    // Keep supervising while the lookup is in flight, so a turn that stays
    // silent gets re-checked every window.
    armSilenceTimer(threadId);
    let lifecycle: string | null = null;
    try {
      lifecycle = (await threadApi.getTurnState(threadId))?.lifecycle ?? null;
    } catch (error) {
      debug(`silence: turn-state lookup failed thread=${threadId} err=%o`, error);
    }
    if (!isMountedRef.current) return;
    const turnEnded =
      (lifecycle === 'completed' || lifecycle === 'interrupted') &&
      !pendingSendsRef.current.has(threadId);
    if (turnEnded) {
      debug(`silence: core reports ${lifecycle} — terminal event missed, settling ${threadId}`);
      clearSilenceTimer(threadId);
      turnSignatureByThreadRef.current.delete(threadId);
      clearThreadStalled(threadId);
      dispatch(clearRuntimeForThread({ threadId }));
      dispatch(clearThreadInferenceActive(threadId));
      void dispatch(loadThreadMessages(threadId));
      return;
    }
    debug(`silence: turn still live (lifecycle=${lifecycle ?? 'none'}) — warning only ${threadId}`);
    markThreadStalled(threadId);
  };

  // Drop every silence timer this component owns when it unmounts.
  //
  // The timer's callback is not inert after teardown: it queries the core and
  // may dispatch `clearRuntimeForThread` / `clearThreadInferenceActive` into
  // shared store state that outlives this component. A page that is gone
  // cannot supervise a turn, so nothing it armed may keep running.
  //
  // Deliberately `[]` — unmount only. Keying this on `selectedThreadId` would
  // clear the timer every time the user switched threads, which is exactly the
  // watchdog this PR exists to arm.
  useEffect(() => {
    isMountedRef.current = true;
    const timers = sendingTimeoutsRef.current;
    return () => {
      isMountedRef.current = false;
      for (const timeout of timers.values()) clearTimeout(timeout);
      timers.clear();
    };
  }, []);

  // A turn this client did not start still needs the 120s watchdog.
  //
  // `armSilenceTimer` is only called on the local send path, so a client that
  // reloads or reconnects mid-turn — hydrating through
  // `fetchAndHydrateTurnState` on thread selection — renders a live-looking
  // "Thinking..." pill with no timer behind it. If the terminal event is then
  // missed, nothing ever clears it: in the observed incident two sockets
  // connected mid-turn, the turn ended 95s later, and the UI still read
  // "Thinking... (15)" 25 minutes on. A core restart heals it today
  // (`mark_all_interrupted` sweeps non-terminal snapshots at startup), which is
  // why it only bites long-lived sessions.
  //
  // Arm only for a turn that is genuinely in flight. A terminal snapshot
  // deletes `inferenceStatusByThread` in the reducer's interrupted/completed
  // branch, so the status check alone already excludes one; the `interrupted`
  // guard is belt-and-braces, so this cannot start warning about silence on a
  // settled thread if that branch ever changes. (`completed` is not a member of
  // `InferenceTurnLifecycle` — the reducer deletes the key instead of storing a
  // terminal value — so there is no such case to guard.)
  //
  // Arming is the whole fix: the rearm effect below iterates
  // `sendingTimeoutsRef` keys, so a thread holding no timer is invisible to it.
  // Once a timer exists, heartbeats (#4270), streaming text and sub-agent tool
  // activity rearm it exactly as for a locally-sent turn, and the
  // done-transition clears it — an inherited turn gets the same treatment as an
  // owned one rather than a second, parallel mechanism.
  useEffect(() => {
    if (!selectedThreadId) return;
    // A local send already armed one. Never replace it: re-arming here would
    // hand the turn a fresh 120s every time this effect re-ran.
    if (sendingTimeoutsRef.current.has(selectedThreadId)) return;
    const lifecycle = inferenceTurnLifecycleByThread[selectedThreadId];
    if (lifecycle === 'interrupted') return;
    // `inferenceStatusByThread` alone is not a complete in-flight test. The
    // hydration reducer only writes it when `iteration > 0 && maxIterations > 0`
    // and deletes it otherwise, so a snapshot that is genuinely running but has
    // not reported its first iteration yet — initial prefill — hydrates with no
    // status entry at all. Keying solely on it would leave exactly that turn
    // without a watchdog, which is the case this effect exists to cover.
    //
    // The lifecycle is written for every non-`completed` snapshot regardless of
    // iteration, so it still identifies a prefill turn. `completed` is not a
    // member of `InferenceTurnLifecycle` (the reducer deletes the key rather
    // than storing a terminal value), so a settled turn leaves both undefined
    // and is correctly skipped.
    const inFlight =
      Boolean(inferenceStatusByThread[selectedThreadId]) ||
      lifecycle === 'started' ||
      lifecycle === 'streaming';
    if (!inFlight) return;
    debug(`inherited in-flight turn on ${selectedThreadId} — arming silence timer`);
    armSilenceTimer(selectedThreadId);
    // `armSilenceTimer` reads only refs and `dispatch`, so it is stable enough
    // to omit; including it would re-run this on every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedThreadId, inferenceStatusByThread, inferenceTurnLifecycleByThread]);

  // Rearm the silence timer on every inference signal for the sending
  // thread. Top-level tool / iteration events bump `inferenceStatusByThread`;
  // pure-text streams (no tools) only bump `streamingAssistantByThread`;
  // sub-agent activity (a delegated `Research`/`Tools Agent`/`Memory Tree`
  // turn whose tools run in a child task) bumps `toolTimelineByThread` without
  // necessarily re-emitting a top-level status change, so it must be watched —
  // otherwise a long sub-agent loop
  // would trip the silence warning mid-run even though the user can see the
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
        clearThreadStalled(threadId);
        continue;
      }
      const changed = !previous || previous.some((value, index) => value !== current[index]);
      if (!changed) continue;
      turnSignatureByThreadRef.current.set(threadId, current);
      // Progress resumed: the "gone quiet" warning no longer applies.
      clearThreadStalled(threadId);
      armSilenceTimer(threadId);
    }
    // armSilenceTimer / clearSilenceTimer are stable (refs + dispatch);
    // depending on the progress maps rearms live timers on every signal.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    inferenceStatusByThread,
    streamingAssistantByThread,
    toolTimelineByThread,
    inferenceHeartbeatByThread,
  ]);

  const handleSlashCommand = (command: string): boolean => {
    const decision = handleComposerSlashCommand(command);
    if (decision.kind === 'not_handled') return false;

    setInputValue('');
    if (decision.kind === 'run_mode') {
      debug('[chat] slash command: run mode -> %s', decision.mode);
      void setRunMode(decision.mode).catch(error => {
        debug('[chat] slash command: set run mode failed: %o', error);
      });
    } else if (decision.kind === 'stop') {
      handleStopGenerationRef.current?.();
    } else {
      void handleCreateNewThread();
    }
    return true;
  };

  const ingestFiles = async (files: FileList | File[] | null) => {
    if (!files) return;
    // Counted from the ref, never from the `attachments` render snapshot: the
    // composer now has three ingest entry points (picker, drop, paste) and two
    // can fire before React re-renders. Both would then seed their budget from
    // the same snapshot and each admit a full quota.
    const admitted = attachmentsRef.current;
    let acceptedFileCount = admitted.filter(attachment => attachment.kind === 'file').length;
    // Images and videos share one image-marker budget (video = its frames), so
    // track consumed markers rather than per-kind counts.
    let acceptedImageMarkers = admitted.reduce(
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
      // Ref first, synchronously: the next queued run counts what this one just
      // took, without waiting for React to commit the state update.
      attachmentsRef.current = [...attachmentsRef.current, result.attachment];
      setAttachments(prev => [...prev, result.attachment]);
    }
  };

  /**
   * Serialises ingest so overlapping gestures cannot both validate against the
   * same budget — a paste landing while a dropped file is still being read, for
   * instance. Each run starts only once the one before it has finished writing
   * `attachmentsRef`, so the budget it counts from is current.
   *
   * The list is copied here, synchronously, and that is load-bearing: every
   * caller hands over a list owned by a DOM event that does not outlive the
   * handler. The picker clears its `FileList` on the next line
   * (`event.target.value = ''`), and a drop's `DataTransfer` is neutered once
   * the handler returns. Reading either from inside the queued continuation
   * finds an empty list — and an empty list produces no attachment and no
   * error, which is a silent failure rather than a visible one.
   */
  const handleAttachFiles = (files: FileList | File[] | null): Promise<void> => {
    const snapshot = files ? Array.from(files) : null;
    const run = ingestQueueRef.current.then(() => ingestFiles(snapshot));
    // The queue must survive a rejected run, or one failure wedges every later
    // attachment. Errors still surface to the caller through `run`.
    ingestQueueRef.current = run.then(
      () => undefined,
      () => undefined
    );
    return run;
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
    const pendingAttachments = attachments.slice();
    const modelOverride = composerModelOverride ?? undefined;
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
    // Silence watchdog: fires only if 120s pass without ANY inference signal
    // (tool call, tool result, iteration start, subagent event, text/thinking
    // delta, heartbeat). The effect below rearms it on every signal for
    // `sendingThreadId`. When it fires it warns and reconciles with the core
    // (`handleSilence`); it never cancels or clears a live turn.
    // Fresh send: clear the previous-status baseline before arming so the
    // first inference signal of this turn isn't misread as a chat-done
    // transition (defined → undefined) left over from the prior turn.
    turnSignatureByThreadRef.current.delete(sendingThreadId);
    clearThreadStalled(sendingThreadId);
    armSilenceTimer(sendingThreadId);
    dispatch(setToolTimelineForThread({ threadId: sendingThreadId, entries: [] }));
    dispatch(beginInferenceTurn({ threadId: sendingThreadId }));
    dispatch(markThreadInferenceActive(sendingThreadId));

    // ── Cloud socket path ─────────────────────────────────────────────────────
    // Primary chat goes through the core over the socket; the core picks the
    // routed provider (managed cloud, BYOK, or a user-run local endpoint such
    // as Ollama configured under Connections → LLM).
    try {
      await chatSend({
        threadId: sendingThreadId,
        message: messageText,
        ...(modelOverride !== undefined ? { model: modelOverride } : {}),
        locale: uiLocale,
        reasoningEffort: composerReasoningEffort,
      });
      trackAnalyticsEvent('chat_message_sent', {
        send_mode: 'standard',
        has_attachments: pendingAttachments.length > 0,
      });
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
      clearThreadStalled(sendingThreadId);
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

  // Queue a FOLLOW-UP on the selected thread while a turn is streaming
  // (queue_mode 'followup'): the backend sends it as a fresh turn once the
  // current turn finishes. We do NOT insert it into the transcript now —
  // appending it mid-stream would persist it BEFORE the in-flight assistant
  // reply (the conversation store is an append log), so the prompt would show
  // out of order on reload. Instead we keep it as a pending follow-up
  // (`queueSlice`), flushed into the transcript (persisted, in order, after the
  // assistant reply) when the turn ends — see `ChatRuntimeProvider`'s done/error
  // paths. What the composer shows is the core's own queue, not this record.
  const handleSendFollowup = async (text?: string) => {
    if (!rustChat || !selectedThreadId) return;
    const threadId = selectedThreadId;
    const normalized = (text ?? inputValue).trim();
    const pendingAttachments = attachments.slice();
    if (!normalized && pendingAttachments.length === 0) return;

    const modelOverride = composerModelOverride ?? undefined;
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
    setSendError(null);
    setAttachError(null);

    try {
      await chatSend({
        threadId,
        message: messageText,
        ...(modelOverride !== undefined ? { model: modelOverride } : {}),
        locale: uiLocale,
        queueMode: 'followup',
        reasoningEffort: composerReasoningEffort,
      });
      // Only clear the composer once the backend has accepted the queue, so a
      // failed send leaves the user's draft + attachments intact to retry.
      setInputValue('');
      setAttachments([]);
      dispatch(pendingFollowupAdded({ threadId, message: followupMessage, text: messageText }));
      trackAnalyticsEvent('chat_message_sent', {
        send_mode: 'followup',
        has_attachments: pendingAttachments.length > 0,
      });
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setSendError(chatSendError('cloud_send_failed', msg));
      // assistant-ui clears its composer after `onNew` resolves. This path
      // handles the transport error locally, so restore the rejected follow-up
      // explicitly instead of letting the user's draft disappear.
      setInputValue(normalized);
    }
  };

  // The composer's Send button (and plain Enter) route to a queued follow-up
  // while the selected thread is streaming, otherwise to a normal send.
  const handleComposerSend = (text?: string): Promise<void> =>
    selectedThreadActive ? handleSendFollowup(text) : handleSendMessage(text);

  handleComposerSendRef.current = handleComposerSend;

  // Cancel the in-flight turn for the selected thread. Shared by the in-composer
  // Stop button (text mode), the ESC-to-interrupt shortcut, and the footer
  // Cancel control (mic-cloud / voice modes) so the cancel path lives in one
  // place.
  //
  // `ChatRuntimeProvider.onCancelled` persists the partial and its processing
  // trail after the core confirms cancellation. Keeping that in one place also
  // covers turns superseded without a local Stop click.
  const handleStopGeneration = useCallback(() => {
    if (!selectedThreadId) {
      debug('[chat] stop generation: no selected thread — noop');
      return;
    }
    const threadId = selectedThreadId;
    debug('[chat] stop generation: thread=%s', threadId);
    void chatCancel(threadId).then(outcome => {
      const accepted = outcome?.accepted === true;
      const turnCancelled = outcome?.turnCancelled === true;
      debug(
        '[chat] stop generation: chatCancel thread=%s accepted=%s turnCancelled=%s',
        threadId,
        accepted,
        turnCancelled
      );
      if (!accepted) return;
      if (!turnCancelled) {
        // The core has nothing running on this thread, so no `cancelled`
        // chat_error will ever arrive to clear the composer. Without this the
        // thread stays "generating" with a Stop button that can never work —
        // e.g. a turn whose terminal event was lost across a reconnect. A send
        // still waiting on its RPC is skipped: its turn may not be registered
        // yet, and its own completion path owns the state.
        if (pendingSendsRef.current.has(threadId)) {
          debug('[chat] stop generation: nothing in flight but send pending thread=%s', threadId);
          return;
        }
        debug(
          '[chat] stop generation: nothing in flight — settling local state thread=%s',
          threadId
        );
        clearSilenceTimer(threadId);
        turnSignatureByThreadRef.current.delete(threadId);
        clearThreadStalled(threadId);
        dispatch(clearRuntimeForThread({ threadId }));
        dispatch(clearThreadInferenceActive(threadId));
        return;
      }
    });
  }, [selectedThreadId, dispatch, clearSilenceTimer, clearThreadStalled]);

  handleStopGenerationRef.current = handleStopGeneration;

  // Claim the selected thread's write path for assistant-ui's runtime, so its
  // `onNew`/`onCancel` forward here instead of reimplementing ~200 lines of
  // send orchestration (see `providers/chatSurfaceHandlers`).
  //
  // `send` routes through `handleComposerSend` — the SAME function the Send
  // button and plain Enter use — so the streaming-vs-idle decision (queued
  // follow-up vs. fresh turn) is made in exactly one place, and
  // `handleSendMessage`'s `evaluateComposerSend` block/allow half runs
  // unchanged. Re-deriving either here is the drift this seam exists to stop.
  useChatSurfaceRegistration(
    selectedThreadId,
    handleComposerSendRef,
    handleStopGenerationRef,
    true
  );

  const handleComposerEscape = useCallback(() => {
    if (!selectedThreadActive) return;
    const composerEmpty = inputValue.trim().length === 0;
    debug(
      '[chat] esc interrupt: thread=%s composerEmpty=%s',
      selectedThreadId ?? 'none',
      composerEmpty
    );
    handleStopGeneration();
    if (composerEmpty) {
      // Restore the last *visible* user prompt (hidden system/injected
      // messages are excluded here to match how the transcript is rendered).
      const lastUserMessage = [...messages]
        .reverse()
        .find(m => m.sender === 'user' && !m.extraMetadata?.hidden);
      const restored = lastUserMessage
        ? parseMessageImages(lastUserMessage.content ?? '').text
        : '';
      if (restored.length > 0) {
        debug('[chat] esc interrupt: restored prompt len=%d', restored.length);
        setInputValue(restored);
      }
    }
  }, [handleStopGeneration, inputValue, messages, selectedThreadActive, selectedThreadId]);

  // The transcript itself renders from the assistant-ui runtime
  // (`AssistantUiChat`). What remains here is what the composer footer and the
  // transcript overlays still need directly.
  const selectedThreadToolTimeline = selectedThreadId
    ? (toolTimelineByThread[selectedThreadId] ?? EMPTY_TOOL_TIMELINE)
    : EMPTY_TOOL_TIMELINE;
  const selectedThreadProcessing = selectedThreadId
    ? (processingByThread[selectedThreadId] ?? EMPTY_PROCESSING)
    : EMPTY_PROCESSING;
  // The command-palette panel describes the whole selected conversation, not
  // only a currently streaming turn. Settled trails are stored per-turn, so
  // combine them with live data here instead of opening an empty panel after a
  // thread is reloaded.
  const selectedThreadProcessSourceEntries = selectedThreadId
    ? [
        ...Object.values(turnTimelinesByThread[selectedThreadId] ?? {}).flat(),
        ...selectedThreadToolTimeline,
      ]
    : EMPTY_TOOL_TIMELINE;
  const selectedThreadProcessSourceTranscript = selectedThreadId
    ? [
        ...Object.values(turnTranscriptsByThread[selectedThreadId] ?? {}).flat(),
        ...selectedThreadProcessing,
      ]
    : EMPTY_PROCESSING;
  // Detached background sub-agents (mode === 'async') spawned in this thread.
  // `TranscriptOverlays` keeps their panel mounted even while its composer
  // shortcut is temporarily hidden.
  const backgroundProcesses = useMemo(
    () => selectBackgroundProcesses(selectedThreadToolTimeline),
    [selectedThreadToolTimeline]
  );
  // Harness work state the agent keeps for this thread — its live todo list
  // and its goal — driven by the dedicated `thread_todos_changed` /
  // `thread_goal_updated` core events (`aui/useThreadTodos.ts` /
  // `aui/useThreadGoal.ts`), primed on thread open by the RPC pair below.
  // Rendered above the composer next to the gate cards so a five-step task
  // shows as a checklist ticking off while the agent works through it.
  useLoadThreadTodos(selectedThreadId ?? null);
  useLoadThreadGoal(selectedThreadId ?? null);
  const liveTodos = useThreadTodos(selectedThreadId ?? null);
  const threadGoal = useThreadGoal(selectedThreadId ?? null);
  // A plan the orchestrator parked for interactive review (request_plan_review
  // gate). When present, the PlanReviewCard renders above the composer and
  // resolves the parked turn.
  const pendingPlanReview = selectedThreadId
    ? (pendingPlanReviewByThread[selectedThreadId] ?? null)
    : null;
  // A candidate automation the agent drafted via `propose_workflow` (issue B4),
  // awaiting the user's Save/Dismiss decision on `WorkflowProposalCard`. Unlike
  // `pendingPlanReview`, the underlying tool call already completed — this
  // just controls whether the card is still showing.
  const pendingWorkflowProposal = selectedThreadId
    ? (pendingWorkflowProposalsByThread[selectedThreadId] ?? null)
    : null;
  // Blocks all composer interaction while a turn is in-flight or Rust chat is unavailable.
  // isSending: the *selected* thread is in-flight (drives selected-thread UI only).
  const composerInteractionBlocked = isComposerInteractionBlocked({
    selectedThreadActive,
    rustChat,
  });

  const isSending = Boolean(
    selectedThreadId &&
    (pendingSendingThreadIds.has(selectedThreadId) ||
      inferenceTurnLifecycleByThread[selectedThreadId] === 'started' ||
      inferenceTurnLifecycleByThread[selectedThreadId] === 'streaming')
  );

  // ── Chat mascot ────────────────────────────────────────────────────────────
  // `null` outside the merged chat surface (embedded sidebars, iOS, the Flows
  // copilot), where no mascot exists and the dock is simply not rendered.
  const chatMascot = useChatMascotOptional();
  // Stable callbacks: the mascot stage subscribes to these, and
  // `handleSendMessage` is re-created every render, so publishing it directly
  // would wake the stage on every keystroke. The ref is already maintained for
  // the dictation handler and always holds the latest send fn.
  const mascotSubmit = useCallback((text: string) => handleSendMessageRef.current?.(text), []);
  const mascotError = useCallback(
    (message: string) => {
      setSendError(chatSendError('voice_transcription', message));
    },
    [setSendError]
  );
  useChatMascotSendBinding(chatMascot, {
    submit: mascotSubmit,
    onError: mascotError,
    // Same guard as the mic-cloud composer below: without `!selectedThreadId` a
    // transcript spoken before a thread exists hits handleSendMessage's early
    // return and is silently dropped — the user spoke into the void.
    disabled: composerInteractionBlocked || isSending || !selectedThreadId,
  });
  const mascotDock = chatMascot ? <ChatMascotDock /> : undefined;

  const filteredThreads = useMemo(() => {
    return threads.filter(t => isThreadVisibleInTab(t, selectedLabel));
  }, [threads, selectedLabel]);

  const sortedThreads = useMemo(() => {
    return [...filteredThreads].sort(
      (a, b) => new Date(b.lastMessageAt).getTime() - new Date(a.lastMessageAt).getTime()
    );
  }, [filteredThreads]);

  const isSidebar = variant === 'sidebar';
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
    <ThreadList
      threads={sortedThreads}
      selectedThreadId={selectedThreadId ?? null}
      onCreateThread={() => void handleCreateNewThread()}
      onSelectThread={id => {
        dispatch(setSelectedThread(id));
        void dispatch(loadThreadMessages(id));
        if (shouldSyncChatRoute) {
          navigate(chatThreadPath(id));
        }
      }}
      resolveTitle={resolveThreadDisplayTitle}
      onRequestDelete={thread =>
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
        })
      }
      editingThreadId={editingThreadId}
      editTitleValue={editTitleValue}
      editTitleInputRef={editTitleInputRef}
      onEditTitleValueChange={setEditTitleValue}
      onStartEditTitle={handleStartEditTitle}
      onCommitTitle={handleCommitTitle}
      onCancelEditTitle={() => {
        ignoreNextTitleBlurRef.current = true;
        setEditingThreadId(null);
      }}
      onBlurTitle={id => {
        if (ignoreNextTitleBlurRef.current) {
          ignoreNextTitleBlurRef.current = false;
          return;
        }
        handleCommitTitle(id);
      }}
    />
  );

  // The two turn-gate cards (a parked plan review and a drafted workflow).
  // Rendered in the composer header of BOTH composers — the text composer and
  // the `mic-cloud` voice composer — which are mutually exclusive, so nothing
  // doubles up.
  const agentGateCards = (
    <>
      {/* Harness work state: the thread goal (as a compact `AgentStatus`
          pill) and the agent's live todo list. Both are read-only progress
          the agent wrote via its tools; they sit above the gate cards so a
          parked decision is always the closest thing to the composer. */}
      {selectedThreadId && threadGoal && (
        <AgentStatus
          data-testid="goal-banner"
          data-goal-status={threadGoal.status}
          state={
            threadGoal.status === 'complete'
              ? 'done'
              : threadGoal.status === 'active'
                ? 'working'
                : 'waiting'
          }
          label={threadGoal.objective}
          trailing={
            <span data-testid="goal-objective" className="text-[10px] tabular-nums">
              {threadGoal.token_budget !== undefined
                ? `${formatTokens(threadGoal.tokens_used)} / ${formatTokens(threadGoal.token_budget)}`
                : formatTokens(threadGoal.tokens_used)}
            </span>
          }
          className="mb-2 self-start"
        />
      )}
      {selectedThreadId && liveTodos && liveTodos.length > 0 && (
        <TodoList
          data-testid="todo-checklist"
          items={toAuiTodoItems(liveTodos)}
          title={t('conversations.todos.title')}
          className="mb-2"
        />
      )}

      {/* Plan-mode review: the orchestrator parked the live turn on a
          thread-scoped plan (request_plan_review gate). Surface it for the
          user to Approve / Reject / send feedback on before anything
          executes. This composer-header render is the pre-C2 fallback: once
          the core sends `tool_call_id` on `plan_review_request`, the SAME
          review renders as part of the `request_plan_review` tool-call part
          (`aui/PlanReviewPart.tsx`) instead, and this block renders nothing
          for it (there is no tool-call part to attach a review WITHOUT a
          tool_call_id, which is why this fallback stays). */}
      {selectedThreadId && pendingPlanReview && !pendingPlanReview.toolCallId && (
        // Key by request id so a re-parked (revised) plan — or a thread switch —
        // remounts the card and resets its local decision/feedback state,
        // matching the ApprovalRequestCard pattern above.
        <PlanReviewCardCore
          key={pendingPlanReview.requestId}
          threadId={selectedThreadId}
          review={pendingPlanReview}
        />
      )}

      {/* Agent-first Workflow authoring (issue B4): the agent drafted a
          candidate automation via `propose_workflow`. The tool only
          validates — it never creates the flow — so this card is the ONLY
          path from proposal to saved automation via "Save & enable"
          (`flows_create`), or the user can Dismiss it outright. */}
      {selectedThreadId && pendingWorkflowProposal && (
        // Keyed by name so a second proposal in the same thread (before the
        // first is resolved) remounts the card and resets its local
        // saving/error state, matching the PlanReviewCard pattern above.
        <WorkflowProposalCard
          key={pendingWorkflowProposal.name}
          threadId={selectedThreadId}
          proposal={pendingWorkflowProposal}
        />
      )}
    </>
  );

  // ── Composer-adjacent surfaces shared by BOTH chat panels ─────────────────
  //
  // Defined once here and rendered by both composers through
  // `assistantComposerHeader` / `assistantComposerFooterExtras`: the text
  // composer's `ComposerHeader` / `ComposerExtras` slots, and the `mic-cloud`
  // voice composer. One definition is the point — a second copy is how they
  // drifted apart before (a rejected send once showed no feedback at all).

  // Shown while the selected thread's live turn has gone quiet past the silence
  // window. Informational: the turn keeps running and clears this itself on its
  // next signal. The wording follows the phase so a long think does not read as
  // a failure.
  const selectedStalledPhase =
    selectedThreadId &&
    stalledThreadIds.has(selectedThreadId) &&
    (selectedThreadActive ||
      inferenceStatusByThread[selectedThreadId] !== undefined ||
      inferenceTurnLifecycleByThread[selectedThreadId] === 'started' ||
      inferenceTurnLifecycleByThread[selectedThreadId] === 'streaming')
      ? (inferenceStatusByThread[selectedThreadId]?.phase ?? 'thinking')
      : null;
  const stallWarningBanner = selectedStalledPhase ? (
    <div className="mb-2" role="status">
      <p
        className="text-xs text-amber-700"
        data-testid="chat-stall-warning"
        data-chat-stall-phase={selectedStalledPhase}>
        {t(
          selectedStalledPhase === 'thinking'
            ? 'chat.stallWarning.thinking'
            : 'chat.stallWarning.working'
        )}
      </p>
    </div>
  ) : null;

  const sendAdvisoryBanner = sendAdvisory ? (
    <div className="flex items-center justify-between mb-2">
      <p className="text-xs text-amber-700" data-chat-send-advisory>
        {sendAdvisory}
      </p>
      <button
        type="button"
        data-analytics-id="chat-send-advisory-dismiss"
        onClick={() => setSendAdvisory(null)}
        className="text-xs text-content-muted hover:text-content-secondary transition-colors ml-2">
        {t('common.dismiss')}
      </button>
    </div>
  ) : null;

  const sendErrorBanner = displayedSendError ? (
    <div className="flex items-center justify-between mb-2">
      <p
        className="text-xs text-coral-500"
        data-testid="chat-send-error"
        data-chat-send-error-code={displayedSendError.code}>
        {displayedSendError.message}
      </p>
      <div className="flex items-center gap-2 shrink-0 ml-2">
        {(displayedSendError.code === 'stt_not_ready' ||
          displayedSendError.code === 'voice_transcription' ||
          displayedSendError.code === 'tts_not_ready' ||
          displayedSendError.code === 'voice_synthesis') && (
          <button
            type="button"
            data-analytics-id="chat-send-error-setup"
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
          type="button"
          data-analytics-id="chat-send-error-dismiss"
          onClick={() => {
            setSendError(null);
            dispatch(clearCreateThreadError());
          }}
          className="text-xs text-content-muted hover:text-content-secondary transition-colors">
          {t('common.dismiss')}
        </button>
      </div>
    </div>
  ) : null;

  // Flow-approval surface (chat): actionable banner(s) for paused tinyflows
  // runs, pushed via the `flow_approval_request` socket event (issue:
  // flow-approval surfacing). Not gated on the selected thread — see the hook
  // call above for why — so every pending request renders regardless of which
  // thread is open.
  const flowApprovalDeck =
    flowApprovalRequests.length > 0 ? (
      <div className="mb-2 flex flex-col gap-2">
        {flowApprovalRequests.map(request => (
          <ApprovalCardAdapter
            key={request.request_id}
            ariaLabel={t('chat.flowApproval.title')}
            title={t('chat.flowApproval.title')}
            subtitle={request.summary || t('chat.flowApproval.fallback')}
            command={request.flow_id}
            toolName={request.tool_name}
            alwaysDecision="approve_always_for_flow"
            alwaysHint={t('chat.flowApproval.approveAlwaysHint')}
            analyticsPrefix="flow-approval-request"
            testId="flow-approval-request-card"
            onDecide={async decision => {
              await decideApproval(request.request_id, decision);
              dismissFlowApprovalRequest(request.request_id);
            }}
          />
        ))}
      </div>
    ) : null;

  // Background-approval surface: parks raised with no chat thread and no flow
  // run. Sits beside the flow deck because it is the same affordance with a
  // different origin, and is likewise not thread-scoped — a pending row has no
  // thread to be scoped to, which is exactly why it had no surface. Only
  // once/deny are offered here (no `alwaysDecision`) — the request arrived
  // from attacker-influenceable content with no interactive session behind
  // it, and a session-wide standing allowlist is the wrong thing to grant
  // from a banner the user did not go looking for.
  const unroutedApprovalDeck =
    unroutedApprovals.length > 0 ? (
      <div className="mb-2 flex flex-col gap-2" data-testid="unrouted-approval-deck">
        {unroutedApprovalError && (
          <p className="text-xs text-destructive" role="alert">
            {unroutedApprovalError}
          </p>
        )}
        {unroutedApprovals.map(approval => (
          <ApprovalCardAdapter
            key={approval.request_id}
            ariaLabel={`Background approval required: ${approval.tool_name}`}
            title={t('chat.approval.title')}
            subtitle={approval.action_summary || approval.tool_name}
            command={approval.tool_name}
            toolName={approval.tool_name}
            expiresAt={approval.expires_at}
            analyticsPrefix="unrouted-approval"
            testId="unrouted-approval-card"
            busy={unroutedDecidingId !== null}
            onDecide={decision => decideUnroutedApproval(approval.request_id, decision)}
          />
        ))}
      </div>
    ) : null;

  // Surface in-flight + failed artifact cards above the composer (#2779).
  // Mirrors the approval-card placement so the user sees the spinner / error
  // without scrolling. `ready` cards are delegated to the header ChatFilesChip
  // panel (#3024) so the chat scroll area isn't permanently occupied —
  // restored decks are listable from the chip on demand.
  //
  // The failed-card Retry button re-dispatches the producing tool via
  // `ai_regenerate` (#3162): the core reloads the persisted creation args and
  // re-runs generation under the original artifact id, so the card swaps back
  // to a spinner in place and then to ready/failed via the socket events.
  const artifactDeckThreadId = selectedThreadId ?? firstActiveThreadId;
  // Only artifacts with NO owning tool call belong in the header deck — one
  // with a `toolCallId` renders inline through its own tool-call card
  // (`MediaAndDocumentCalls.tsx`) instead, per the `ArtifactCardAdapter` doc.
  const liveArtifacts = artifactDeckThreadId
    ? (artifactsByThread[artifactDeckThreadId] ?? []).filter(
        a => a.status !== 'ready' && !a.toolCallId
      )
    : [];
  const liveArtifactDeck =
    liveArtifacts.length > 0 && artifactDeckThreadId ? (
      <div className="mb-2 flex flex-col gap-2">
        {liveArtifacts.map(artifact => (
          <ArtifactCardAdapter
            key={artifact.artifactId}
            artifact={artifact}
            onRetry={id => {
              void aiRegenerate(id, artifactDeckThreadId).catch(err => {
                console.warn('[artifact] regenerate failed:', err);
              });
            }}
          />
        ))}
      </div>
    ) : null;

  const chatFilesChip =
    (selectedThreadId ?? firstActiveThreadId) ? (
      <ChatFilesChip threadId={(selectedThreadId ?? firstActiveThreadId) as string} />
    ) : null;

  const assistantComposerHeader = (
    <>
      {/* Turn gates first: a parked plan review and a drafted workflow both
          block progress until the user decides, so they sit above the transient
          attach error and the queued-followup strip. `ComposerHeader` is the
          only host slot assistant-ui threads arbitrary React through
          (`thread.tsx`), and it renders directly above the input. */}
      {agentGateCards}
      {/* Paused tinyflows runs block the same way a plan gate does — the
          banner carries the only Approve/Reject affordance — so they belong
          with the gates, above the transient banners. */}
      {flowApprovalDeck}
      {/* Same reasoning, different origin: a background trigger's park blocks
          until someone answers it, and this is the only place it is ever
          asked. */}
      {unroutedApprovalDeck}
      {attachError && (
        <div className="rounded-lg border border-coral-200 bg-coral-50 px-3 py-2">
          <p className="text-xs text-coral-500" data-chat-send-error-code={attachError.code}>
            {attachError.message}
          </p>
        </div>
      )}
      {/* A rejected send is the one failure the user cannot diagnose from the
          transcript: nothing is added to it. Without this the composer simply
          swallowed the message. */}
      {sendErrorBanner}
      {stallWarningBanner}
      {sendAdvisoryBanner}
      {liveArtifactDeck}
      {/* The core's run queue for this thread; renders nothing while empty. */}
      <ComposerMessageQueue />
    </>
  );

  // Left-hand controls in the assistant-ui composer toolbar.
  const assistantComposerFooterExtras = (
    <>
      <ReasoningEffortPicker
        value={composerReasoningEffort}
        onChange={applyComposerReasoningEffort}
      />
      {chatFilesChip}
    </>
  );

  // The mic-first (`mic-cloud`) composer. It replaces only the text composer:
  // the transcript above it is the same assistant-ui `Thread` as text mode, so
  // voice and text are one surface with two inputs. It carries the same header
  // cards as the text composer, plus the voice-only controls: a footer Cancel
  // (there is no in-box Stop button without a text composer), the mascot dock,
  // the host's voice-chat control and the push-to-talk mic.
  const voiceComposer =
    composer === 'mic-cloud' ? (
      <div className="flex flex-col gap-2" data-testid="voice-composer">
        {assistantComposerHeader}
        {isSending && rustChat && (
          <div className="flex justify-start px-1">
            <button
              type="button"
              data-analytics-id="chat-cancel-generation"
              onClick={handleStopGeneration}
              className="text-xs text-content-muted transition-colors hover:text-content-secondary">
              {t('common.cancel')}
            </button>
          </div>
        )}
        {/* `relative` so the mascot dock (absolute, `bottom-full`) anchors here. */}
        <div className="relative flex flex-col items-center gap-3 py-1">
          {mascotDock}
          {voiceChatControl}
          {showMicComposer && (
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
          )}
        </div>
        {!isSidebar && selectedThreadParent && (
          <button
            type="button"
            data-analytics-id="chat-header-back-to-parent-thread"
            onClick={() => {
              dispatch(setSelectedThread(selectedThreadParent.id));
              void dispatch(loadThreadMessages(selectedThreadParent.id));
              navigate(chatThreadPath(selectedThreadParent.id));
            }}
            className="flex items-center gap-1 rounded px-1 text-[11px] font-medium text-primary-600 hover:text-primary-700 hover:underline focus:outline-hidden focus-visible:ring-2 focus-visible:ring-primary-300"
            data-testid="worker-thread-back-to-parent">
            <span aria-hidden="true">←</span>
            <span className="max-w-[16rem] truncate">
              {t('chat.backToThread').replace('{title}', selectedThreadParent.title)}
            </span>
          </button>
        )}
        <div className="flex items-center justify-between gap-2">
          <ContextUsage
            threadId={selectedThreadId}
            modelContextWindow={composerModelContextWindow}
          />
          {!isSidebar && (
            <div className="flex shrink-0 items-center gap-2">{assistantComposerFooterExtras}</div>
          )}
        </div>
      </div>
    ) : undefined;

  const assistantUiMainPanel = (
    <div
      className={
        isSidebar
          ? 'flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden border-l border-line bg-surface'
          : 'flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden'
      }>
      <AssistantUiChat
        model={composerModelOverride ?? resolvedModel ?? CHAT_MODEL_HINT}
        modelContextWindow={composerModelContextWindow}
        composerHeader={assistantComposerHeader}
        composerFooterExtras={assistantComposerFooterExtras}
        composerReplacement={voiceComposer}
        inputValue={inputValue}
        onInputValueChange={setInputValue}
        onEscape={handleComposerEscape}
        attachments={attachments}
        onAttachFiles={handleAttachFiles}
        onRemoveAttachment={id => setAttachments(previous => previous.filter(a => a.id !== id))}
        maxAttachments={ATTACHMENT_MAX_IMAGES + ATTACHMENT_MAX_FILES}
        attachmentsEnabled={CHAT_ATTACHMENTS_ENABLED}
        attachmentInteractionBlocked={composerInteractionBlocked || isSending}
        onAttachmentOnlySend={() => void handleComposerSend()}
        // Idle-composer shortcut to the full-bleed mascot stage. Chat and Human
        // share one mascot (mascotSlice), so this is a change of venue for the
        // same conversation partner, not a second one.
        onOpenHumanMode={() => navigate('/human')}
        onSwitchToMicCloud={() => setComposerOverride('mic-cloud')}
        onModelChange={applyComposerModel}
      />
      {/* The transcript-local overlays: background processes and the Agent
          Process Source panel. Mounted beside the Thread (not inside it)
          because each is its own overlay, positioned against the viewport. */}
      <TranscriptOverlays
        threadId={selectedThreadId ?? null}
        entries={selectedThreadProcessSourceEntries}
        transcript={selectedThreadProcessSourceTranscript}
        backgroundProcesses={backgroundProcesses}
        showBackgroundProcesses={showBackgroundProcesses}
        onCloseBackgroundProcesses={() => setShowBackgroundProcesses(false)}
        showProcessSource={showProcessSource}
        onCloseProcessSource={() => setShowProcessSource(false)}
      />
    </div>
  );
  // One transcript for both composers: `mic-cloud` only swaps the input (see
  // `voiceComposer`), so the voice surface is assistant-ui end to end too.
  const mainPanel = assistantUiMainPanel;

  return (
    <div
      className={
        isSidebar
          ? 'h-full relative z-10 flex overflow-hidden'
          : // No background of its own.
            // The old bg-surface/70 with a dark-mode black/40 override was a
            // translucent tint over the app canvas, which composed to pure
            // black in dark — the colour the composer fade below hardcoded. On
            // the opaque content card it instead composes to an un-tokened
            // ~#0e0e0e that nothing else in the app can name or match, so the
            // fade could never line up. The page now simply *is* the card's
            // surface, and the fade matches by construction.
            'h-full relative z-10 flex justify-center overflow-hidden'
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
    </div>
  );
};

export default Conversations;

/**
 * Embeddable variant — same component, page layout (floating centered
 * card). Mounted inside /accounts when the Agent entry is selected.
 */
export const ConversationsPage = () => <Conversations variant="page" />;
