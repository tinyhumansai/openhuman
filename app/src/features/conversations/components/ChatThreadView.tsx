import {
  forwardRef,
  Fragment,
  type ReactNode,
  useEffect,
  useImperativeHandle,
  useMemo,
  useState,
} from 'react';

import { useStickToBottom } from '../../../hooks/useStickToBottom';
import { parseMessageImages } from '../../../lib/attachments';
import { unwrapToolCallEnvelope } from '../../../lib/chat/toolCallEnvelope';
import { useT } from '../../../lib/i18n/I18nContext';
import { subagentApi } from '../../../services/api/subagentApi';
import {
  markSubagentCancelled,
  type ProcessingTranscriptItem,
  type ToolTimelineEntry,
} from '../../../store/chatRuntimeSlice';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import { persistReaction } from '../../../store/threadSlice';
import type { ThreadMessage } from '../../../types/thread';
import { splitAgentMessageIntoBubbles } from '../../../utils/agentMessageBubbles';
import { formatTimelineEntry } from '../../../utils/toolTimelineFormatting';
import { ShareMessageButton } from '../../share/ShareMessageButton';
import { buildThreadTimeline } from '../timeline/selectors';
import { type AgentBubblePosition, formatRelativeTime } from '../utils/format';
import { AgentMessageBubble, AgentMessageText, BubbleMarkdown } from './AgentMessageBubble';
import { AgentProcessSourcePanel } from './AgentProcessSourcePanel';
import { BackgroundProcessesPanel, selectBackgroundProcesses } from './BackgroundProcessesPanel';
import { CitationChips, type MessageCitation } from './CitationChips';
import { InterruptedAnswer } from './InterruptedAnswer';
import { PastTurnInsights } from './PastTurnInsights';
import { SubagentDrawer } from './SubagentDrawer';
import { ToolTimelineBlock } from './ToolTimelineBlock';

/** Maximum trailing characters rendered in the live-streaming assistant
 *  preview bubble. The full response is revealed via `addInferenceResponse`
 *  on `chat_done` — this is purely a ticker-tape affordance to signal
 *  progress without jumping the scroll position as tokens arrive. */
const STREAMING_PREVIEW_CHARS = 120;

// Matches only well-formed base64 image data URIs — guards against an
// `<img src>` XSS vector if a persisted message ever carried a crafted
// value in `attachmentDataUris`/legacy `[IMAGE:...]` markers.
const SAFE_IMAGE_DATA_URI_RE =
  /^data:(image\/(?:png|jpe?g|gif|webp|bmp));base64,([a-z0-9+/=\s]+)$/i;
const EMPTY_IMAGE_SRC = 'data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==';

function imageDataUriToObjectUrl(src: string): string | null {
  const match = SAFE_IMAGE_DATA_URI_RE.exec(src);
  if (!match) return null;
  try {
    const mime = match[1];
    const binary = atob(match[2].replace(/\s/g, ''));
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i += 1) {
      bytes[i] = binary.charCodeAt(i);
    }
    return URL.createObjectURL(new Blob([bytes], { type: mime }));
  } catch {
    return null;
  }
}

function AttachmentImage({ dataUri }: { dataUri: string }) {
  const [objectUrl, setObjectUrl] = useState<string | null>(null);

  useEffect(() => {
    const nextUrl = imageDataUriToObjectUrl(dataUri);
    setObjectUrl(nextUrl);
    return () => {
      if (nextUrl) URL.revokeObjectURL(nextUrl);
    };
  }, [dataUri]);

  return (
    <img
      src={objectUrl ?? EMPTY_IMAGE_SRC}
      alt=""
      className="max-w-[200px] max-h-[200px] rounded-2xl object-cover"
    />
  );
}

// Stable empty reference for a thread with no persisted messages yet, so the
// selector below keeps the same identity when the slice field is absent
// (narrow test stores) or the thread hasn't hydrated any messages.
const EMPTY_MESSAGES: ThreadMessage[] = [];

// Stable empty reference so the per-thread past-turn timelines map keeps the
// same identity when the slice field is absent (narrow test stores).
const EMPTY_TURN_TIMELINES: Record<string, ToolTimelineEntry[]> = {};
// Sibling stable empty for the per-thread past-turn processing transcripts map
// (restore-fidelity fix 1).
const EMPTY_TURN_TRANSCRIPTS: Record<string, ProcessingTranscriptItem[]> = {};
// Stable empty transcript for a past turn that has tool rows but no persisted
// reasoning/narration trail (legacy snapshot), so `PastTurnInsights` falls back
// to the tool-only view without allocating a fresh array each render.
const EMPTY_TRANSCRIPT: ProcessingTranscriptItem[] = [];
// Stable empty tool-row list for a transcript-only past turn (agent thought /
// narrated but ran no tools).
const EMPTY_TRANSCRIPT_ENTRIES: ToolTimelineEntry[] = [];

export interface ChatThreadViewHandle {
  /** Opens the detached background sub-agents panel — called from the host
   *  header's background-processes badge (`Conversations`' own copy of the
   *  badge, which needs to stay in the header so it renders regardless of
   *  scroll position). */
  openBackgroundProcesses(): void;
}

export interface ChatThreadViewProps {
  /** Thread whose transcript is rendered. `null` renders the empty state
   *  (no thread selected). */
  threadId: string | null;
  /** `page` (default) is the home chat's floating-composer layout. `sidebar`
   *  drops the bottom-padding-reservation trick (the composer is in normal
   *  flow) and always applies a flat `pb-4`. */
  variant?: 'page' | 'sidebar';
  /** Page variant floats its footer over the scroll area; the host passes
   *  its measured `composerFooterHeight + 16` so the message list reserves
   *  matching bottom padding. Sidebar variant omits this (undefined — the
   *  message list falls back to a flat `pb-4`). */
  bottomPadding?: number;
  /** The host's footer has pinned content (e.g. home's task board) that
   *  should keep the scroll container in "has content" layout even when
   *  there are no visible messages and no live agent activity yet. */
  hasFooterContent?: boolean;
  /** The host's thread-load state, so the loading skeleton renders while a
   *  thread's messages are being fetched. Omit for hosts that hydrate
   *  synchronously (no skeleton). */
  isLoading?: boolean;
  loadError?: string | null;
  /** Rendered in place of the transcript when the thread has no visible
   *  messages, no footer content, and no live agent activity. */
  emptyContent?: ReactNode;
  /** Agent display name threaded into the assistant message's share button. */
  shareAgentName?: string;
  /** Reset key for the auto-stick-to-bottom scroll behavior (see
   *  `useStickToBottom`). Pass a value that changes when the surrounding
   *  layout/route changes shape (home passes `location.pathname`). */
  scrollResetKey?: string;
  /** ORed into the in-flight check alongside the persisted inference-turn
   *  lifecycle — hosts that track their own "send dispatched, not yet
   *  accepted" window (home's `pendingSendingThreadIds`) pass it here so a
   *  send appears in-flight immediately, before the first socket event
   *  lands. */
  pendingSendActive?: boolean;
}

/**
 * The chat transcript scroll body: message list (with reactions, citations,
 * copy/share affordances, past-turn insight trails, live streaming preview,
 * inference status line, and the "Agentic task insights" panel), plus the
 * background-processes / sub-agent-drawer / agent-process-source modals that
 * are driven entirely by transcript-local state.
 *
 * Extracted from `Conversations.tsx` (the home chat) so a second surface
 * (the Workflow Copilot, on its own dedicated thread) can reuse the exact
 * same rich rendering. Everything here is keyed off the `threadId` prop
 * rather than the global `state.thread.selectedThreadId` — all state reads
 * are per-thread Redux slices already keyed by thread id.
 */
export const ChatThreadView = forwardRef<ChatThreadViewHandle, ChatThreadViewProps>(
  (
    {
      threadId,
      variant = 'page',
      bottomPadding,
      hasFooterContent = false,
      isLoading = false,
      loadError = null,
      emptyContent = null,
      shareAgentName = 'OpenHuman',
      scrollResetKey = 'chat-thread-view',
      pendingSendActive = false,
    },
    ref
  ) => {
    const { t } = useT();
    const dispatch = useAppDispatch();

    const isSidebar = variant === 'sidebar';

    const messages = useAppSelector(state =>
      threadId ? (state.thread.messagesByThreadId[threadId] ?? EMPTY_MESSAGES) : EMPTY_MESSAGES
    );
    const toolTimelineByThread = useAppSelector(state => state.chatRuntime.toolTimelineByThread);
    const turnTimelinesByThread = useAppSelector(state => state.chatRuntime.turnTimelinesByThread);
    const turnTranscriptsByThread = useAppSelector(
      state => state.chatRuntime.turnTranscriptsByThread
    );
    const interruptedAssistantByThread = useAppSelector(
      state => state.chatRuntime.interruptedAssistantByThread
    );
    const processingByThread = useAppSelector(state => state.chatRuntime.processingByThread);
    const inferenceStatusByThread = useAppSelector(
      state => state.chatRuntime.inferenceStatusByThread
    );
    const streamingAssistantByThread = useAppSelector(
      state => state.chatRuntime.streamingAssistantByThread
    );
    const parallelStreamsByThread = useAppSelector(
      state => state.chatRuntime.parallelStreamsByThread
    );
    const inferenceTurnLifecycleByThread = useAppSelector(
      state => state.chatRuntime.inferenceTurnLifecycleByThread
    );
    const agentMessageViewMode = useAppSelector(
      state => state.theme?.agentMessageViewMode ?? 'bubbles'
    );
    // When ON, the verbose per-agent "Agentic task insights" timeline is hidden
    // from chat; a compact blinking "Processing" link (and the existing message
    // bubble loading) stand in for it, with the full run one click away in the
    // Agent Process Source side panel. See themeSlice.hideAgentInsights.
    const hideAgentInsights = useAppSelector(state => state.theme?.hideAgentInsights ?? false);

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
    const [reactionPickerMsgId, setReactionPickerMsgId] = useState<string | null>(null);

    useImperativeHandle(ref, () => ({
      openBackgroundProcesses: () => setShowBackgroundProcesses(true),
    }));

    const { containerRef: messagesContainerRef, endRef: messagesEndRef } = useStickToBottom(
      messages,
      threadId,
      scrollResetKey
    );

    const handleCopyMessage = async (messageId: string, content: string) => {
      try {
        await navigator.clipboard.writeText(content);
        setCopiedMessageId(messageId);
        setTimeout(() => setCopiedMessageId(null), 1500);
      } catch {
        // Clipboard API not available — silently fail
      }
    };

    const selectedThreadToolTimeline = threadId ? (toolTimelineByThread[threadId] ?? []) : [];
    const selectedThreadProcessing = threadId ? (processingByThread[threadId] ?? []) : [];
    // Detached background sub-agents (mode === 'async') spawned in this thread.
    const backgroundProcesses = useMemo(
      () => selectBackgroundProcesses(selectedThreadToolTimeline),
      [selectedThreadToolTimeline]
    );
    // Re-derive the open subagent's live activity (and its row status) from the
    // timeline on every render so the drawer streams token-by-token as
    // subagent_text_delta / subagent_thinking_delta events land in Redux.
    const openSubagentEntry = openSubagentTaskId
      ? selectedThreadToolTimeline.find(entry => entry.subagent?.taskId === openSubagentTaskId)
      : undefined;
    const visibleMessages = messages.filter(msg => !msg.extraMetadata?.hidden);
    const hasVisibleMessages = visibleMessages.length > 0;
    const latestVisibleMessage = visibleMessages[visibleMessages.length - 1] ?? null;
    const latestVisibleAgentMessage = [...visibleMessages]
      .reverse()
      .find(msg => msg.sender === 'agent');
    // Message list sourced from the unified timeline projection — the single
    // source of render order (see `docs/plans/conversations-timeline-refactor.md`
    // Phase 2). With the streaming/tool inputs omitted the projection yields
    // exactly the visible messages in order; the tool-timeline block and streaming
    // previews stay anchored inline below, so the rendered DOM is unchanged. This
    // routes the live render loop through the projection ahead of the per-turn
    // grouping in Phase 5.
    const timelineMessages = useMemo(
      () =>
        buildThreadTimeline({
          threadId: threadId ?? '',
          messages: visibleMessages,
          toolTimeline: [],
          streaming: null,
          parallelStreams: [],
          hideAgentInsights: false,
        })
          .map(item => ('message' in item ? item.message : null))
          .filter((message): message is ThreadMessage => message !== null),
      [threadId, visibleMessages]
    );
    // Past-turn tool timelines (Phase 5): map the first assistant message of each
    // older settled turn to that turn's timeline, so each past answer renders its
    // own collapsed process trail above it. The latest turn is excluded upstream
    // (it renders as the live "agent insights" anchor), so there is no double
    // render. Empty for legacy messages without a `requestId`.
    const selectedThreadTurnTimelines = threadId
      ? (turnTimelinesByThread[threadId] ?? EMPTY_TURN_TIMELINES)
      : EMPTY_TURN_TIMELINES;
    // Sibling map: each past turn's persisted reasoning/narration trail, so a
    // reopened turn replays its thoughts, not just its tool cards (fix 1).
    const selectedThreadTurnTranscripts = threadId
      ? (turnTranscriptsByThread[threadId] ?? EMPTY_TURN_TRANSCRIPTS)
      : EMPTY_TURN_TRANSCRIPTS;
    const pastTurnAnchors = useMemo(() => {
      const anchors: Record<
        string,
        { entries: ToolTimelineEntry[]; transcript: ProcessingTranscriptItem[] }
      > = {};
      const seen = new Set<string>();
      for (const msg of timelineMessages) {
        if (msg.sender !== 'agent') continue;
        const requestId = msg.extraMetadata?.requestId;
        if (typeof requestId !== 'string' || seen.has(requestId)) continue;
        const entries = selectedThreadTurnTimelines[requestId] ?? EMPTY_TRANSCRIPT_ENTRIES;
        const transcript = selectedThreadTurnTranscripts[requestId] ?? EMPTY_TRANSCRIPT;
        // Anchor the turn when it has EITHER tool rows OR a reasoning/narration
        // trail — a tool-less turn (agent only thought/narrated) must still
        // render its restored thoughts above its answer (fix 1).
        if (entries.length > 0 || transcript.length > 0) {
          anchors[msg.id] = { entries, transcript };
          seen.add(requestId);
        }
      }
      return anchors;
    }, [timelineMessages, selectedThreadTurnTimelines, selectedThreadTurnTranscripts]);
    const activeSubagentTimelineEntry = selectedThreadToolTimeline.find(
      entry => entry.status === 'running' && entry.name.startsWith('subagent:')
    );
    const activeToolTimelineEntry = [...selectedThreadToolTimeline]
      .reverse()
      .find(entry => entry.status === 'running' && !entry.name.startsWith('subagent:'));
    const selectedInferenceStatus = threadId ? (inferenceStatusByThread[threadId] ?? null) : null;
    const selectedStreamingAssistant = threadId
      ? (streamingAssistantByThread[threadId] ?? null)
      : null;
    // The partial reply an interrupted turn left behind (restore-fidelity fix 2):
    // surfaced as a settled, marked-interrupted bubble on restore so a turn that
    // crashed mid-answer keeps its visible work instead of rendering blank.
    const selectedInterruptedAssistant = threadId
      ? (interruptedAssistantByThread[threadId] ?? null)
      : null;
    // Live streams for concurrent parallel (forked) turns on the selected thread,
    // rendered as separate interleaved branch bubbles.
    const selectedParallelStreams = threadId
      ? Object.values(parallelStreamsByThread[threadId] ?? {})
      : [];

    const isSending = Boolean(
      threadId &&
      (pendingSendActive ||
        inferenceTurnLifecycleByThread[threadId] === 'started' ||
        inferenceTurnLifecycleByThread[threadId] === 'streaming')
    );
    const shouldRenderTimelineBeforeLatestAgentMessage =
      selectedThreadToolTimeline.length > 0 && !isSending && Boolean(latestVisibleAgentMessage);

    // Live agent activity that must stay visible even before the thread's
    // message history has loaded: an in-flight turn, recorded tool steps, a
    // processing transcript, or streamed prose. Without this, switching to a
    // thread mid-turn rendered a blank pane (the message list is gated on
    // `hasVisibleMessages`) until `loadThreadMessages` resolved — tool calls and
    // streaming output silently invisible despite landing in Redux.
    const hasLiveAgentActivity =
      isSending ||
      selectedThreadToolTimeline.length > 0 ||
      selectedThreadProcessing.length > 0 ||
      Boolean(selectedStreamingAssistant) ||
      // An interrupted turn's restored partial answer must surface too, even
      // before the durable message history loads (restore-fidelity fix 2).
      Boolean(selectedInterruptedAssistant);

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
              // Reuse `isSending` rather than a raw `in` membership check on
              // `inferenceTurnLifecycleByThread`: that map also carries
              // `'interrupted'` entries (a turn that crashed mid-flight in a
              // PRIOR core process, written by `hydrateRuntimeFromSnapshot` on
              // cold boot) which have no live driver and must NOT read as an
              // active turn, or stale disclosure state leaks into a later
              // retry. `isSending` already excludes it (only `'started'` /
              // `'streaming'`, same as this component's own live-turn checks).
              turnActive={isSending}
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

    // Standalone fallback slot (rendered once, below all messages) for the
    // rare thread with no user message at all (e.g. a proactive-only run), so
    // `agentInsights` is never unreachable. This slot sits at a fixed JSX
    // position with no per-thread key of its own, so switching directly
    // between two threads that both hit this fallback (e.g. two proactive-only
    // threads) would otherwise reuse the same `ToolTimelineBlock` instance
    // instead of remounting it — leaking its sticky `userOverrideOpen`
    // disclosure state from the old thread into the new one (flagged in
    // review on #4942). Keying on thread id forces a clean remount on every
    // thread switch, matching the `key={msg.id}` pattern used for the in-flow
    // timeline above.
    const proactiveInsightsFallback = (() => {
      if (lastUserMessageId) return null;
      return <Fragment key={threadId ?? 'none'}>{agentInsights}</Fragment>;
    })();

    const hasContent = hasVisibleMessages || hasFooterContent || hasLiveAgentActivity;

    return (
      <>
        <div
          ref={messagesContainerRef}
          data-testid="chat-messages-scroll"
          // Full-width scroll (scrollbar hugs the window edge); inner content is
          // centered and width-capped per branch below. `min-h-0` lets this
          // basis-0 flex child shrink to 0 so the composer footer can take the
          // space (and scroll) on short windows (#3785).
          className="flex-1 min-h-0 overflow-y-auto">
          {isLoading ? (
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
          ) : loadError ? (
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
              <p className="text-xs text-content-secondary mb-3 text-center">{loadError}</p>
              <button
                type="button"
                data-analytics-id="chat-messages-reload"
                onClick={() => window.location.reload()}
                className="text-xs text-primary-400 hover:text-primary-300 transition-colors">
                {t('common.reload')}
              </button>
            </div>
          ) : hasContent ? (
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
              style={bottomPadding !== undefined ? { paddingBottom: bottomPadding } : undefined}>
              {timelineMessages.map(msg => {
                const isAgentTextMode = msg.sender === 'agent' && agentMessageViewMode === 'text';
                // B25: an agent turn that both talks AND calls a tool can leak
                // the provider wire-format `{ content, tool_calls }` JSON
                // envelope as its raw `content` (see `unwrapToolCallEnvelope`).
                // Unwrap agent messages to the human text so no surface (home
                // chat OR the workflow copilot, which shares this renderer)
                // ever paints raw JSON. Shape-based + a strict passthrough for
                // ordinary prose, so this is a no-op for every non-envelope
                // message — the tool activity itself renders via the timeline,
                // so the extracted tool names are intentionally dropped here.
                const displayContent =
                  msg.sender === 'agent'
                    ? unwrapToolCallEnvelope(msg.content ?? '').text
                    : (msg.content ?? '');
                // Parsed once per message: for current messages (extraMetadata
                // present, or agent messages) the content already has no markers,
                // so this is a no-op. For legacy persisted user messages with raw
                // [IMAGE:...]/[FILE:...] markers and no extraMetadata, this is
                // what keeps the marker text out of both the rendered bubble and
                // the copy-to-clipboard action.
                const parsedContent = parseMessageImages(displayContent);
                const pastTurn = pastTurnAnchors[msg.id];
                return (
                  <Fragment key={msg.id}>
                    {/* Past-turn process trail (Phase 5 + restore-fidelity fix 1):
                        each older settled turn's interleaved reasoning/narration +
                        tool steps (and restored sub-agent transcripts), collapsed,
                        above the answer it produced. Falls back to tool-cards-only
                        for legacy snapshots with no persisted transcript. */}
                    {pastTurn ? (
                      <div data-testid="past-turn-insights">
                        <PastTurnInsights
                          entries={pastTurn.entries}
                          transcript={pastTurn.transcript}
                        />
                      </div>
                    ) : null}
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
                                  <AgentMessageText content={displayContent} />
                                ) : (
                                  splitAgentMessageIntoBubbles(displayContent).map(
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
                                      (msg.extraMetadata?.myReactions as string[] | undefined) ??
                                      [];
                                    const pickerOpen = reactionPickerMsgId === msg.id;
                                    return (
                                      <div className="absolute -bottom-2 left-3 z-10 flex items-center gap-1">
                                        {myReactions.map(emoji => (
                                          <button
                                            key={emoji}
                                            type="button"
                                            data-analytics-id="chat-message-reaction-remove"
                                            onClick={() =>
                                              threadId &&
                                              void dispatch(
                                                persistReaction({
                                                  threadId,
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
                                                  if (threadId) {
                                                    void dispatch(
                                                      persistReaction({
                                                        threadId,
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
                              {/* Stopped marker (#4862): the partial reply that was
                                  preserved when the user hit Stop / ESC mid-stream. */}
                              {msg.extraMetadata?.stopped === true && (
                                <p
                                  data-testid="stopped-marker"
                                  className="flex items-center gap-1 px-1 text-[10px] font-medium text-content-faint">
                                  <svg
                                    className="h-2.5 w-2.5"
                                    fill="currentColor"
                                    viewBox="0 0 24 24"
                                    aria-hidden>
                                    <rect x="6" y="6" width="12" height="12" rx="1.5" />
                                  </svg>
                                  {t('chat.stoppedByUser')}
                                </p>
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
                                <p className="px-1 text-[10px] text-content-faint">
                                  {formatRelativeTime(msg.createdAt)}
                                </p>
                              )}
                            </div>
                          ) : (
                            <div className="flex flex-col items-end gap-1">
                              {(() => {
                                const displayText = parsedContent.text;
                                const dataUris = (
                                  Array.isArray(msg.extraMetadata?.attachmentDataUris)
                                    ? (msg.extraMetadata.attachmentDataUris as string[])
                                    : parsedContent.dataUris
                                ).filter(src => SAFE_IMAGE_DATA_URI_RE.test(src));
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
                                          <AttachmentImage key={i} dataUri={uri} />
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
                                            <span className="truncate font-medium">
                                              {video.name}
                                            </span>
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
                          {msg.sender === 'agent' && (
                            <ShareMessageButton
                              content={parsedContent.text}
                              agentName={shareAgentName}
                              threadId={threadId ?? undefined}
                              className={`absolute top-6 ${isAgentTextMode ? 'right-0' : '-right-8'}`}
                            />
                          )}
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
                  selectedStreamingAssistant.content.length > 0) && (
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
                      {selectedStreamingAssistant.content.length > 0 && (
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
              {/* Interrupted turn's partial answer (restore-fidelity fix 2):
                    a settled, marked-interrupted bubble surfaced on restore. Only
                    when NOT streaming live (the buffer is cleared by any live turn
                    in the slice; this guard is belt-and-braces). */}
              {!isSending && selectedInterruptedAssistant ? (
                <InterruptedAnswer
                  content={selectedInterruptedAssistant.content}
                  thinking={selectedInterruptedAssistant.thinking}
                />
              ) : null}
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
                              seq: 0,
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
                              seq: 0,
                              status: 'running',
                            }
                          ).title
                        }...`}
                    </span>
                  </div>
                )}
              {/* The "Agentic task insights" panel is rendered inline *above* the
                  latest answer (right after the latest turn's user message) so
                  processing reads before the result. `proactiveInsightsFallback`
                  (defined above, near `agentInsights`) covers the rare thread
                  with no user message at all — see its doc comment for the
                  per-thread keying that fix keeps this remount-safe. */}
              {proactiveInsightsFallback}
              <div ref={messagesEndRef} />
            </div>
          ) : (
            emptyContent
          )}
        </div>
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
            openSubagentEntry?.subagent && threadId
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
                    dispatch(markSubagentCancelled({ threadId, taskId: result.taskId }));
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
      </>
    );
  }
);

ChatThreadView.displayName = 'ChatThreadView';
