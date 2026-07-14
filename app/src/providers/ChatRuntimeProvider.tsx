import debug from 'debug';
import { useCallback, useEffect, useRef } from 'react';

import { requestUsageRefresh } from '../hooks/usageRefresh';
import { useRefetchSnapshotOnTurnEnd } from '../hooks/useRefetchSnapshotOnTurnEnd';
import {
  createSkillToolChainLatencyTracker,
  SKILL_TOOL_CHAIN_TARGET_MS,
} from '../lib/ai/skillToolChainLatency';
import { ingestRuntimeErrorSignal } from '../lib/userErrors/report';
import {
  type ChatApprovalRequestEvent,
  type ChatDoneEvent,
  type ChatInferenceHeartbeatEvent,
  type ChatInferenceStartEvent,
  type ChatInterimEvent,
  type ChatIterationStartEvent,
  type ChatPlanReviewRequestEvent,
  type ChatSegmentEvent,
  type ChatSubagentDoneEvent,
  type ChatSubagentTextDeltaEvent,
  type ChatSubagentThinkingDeltaEvent,
  type ChatTaskBoardUpdatedEvent,
  type ChatToolCallEvent,
  type ChatToolResultEvent,
  type ExternalTransferPendingEvent,
  type ProactiveMessageEvent,
  segmentText,
  subscribeChatEvents,
} from '../services/chatService';
import { store } from '../store';
import {
  appendSubagentStreamDelta,
  bumpInferenceHeartbeatForThread,
  clearInferenceStatusForThread,
  clearParallelRequest,
  clearPendingApprovalForThread,
  clearPendingPlanReviewForThread,
  clearProcessingForThread,
  clearStreamingAssistantForThread,
  endInferenceTurn,
  markInferenceTurnStreaming,
  parseToolFailure,
  recordChatTurnUsage,
  recordSubagentTranscriptTool,
  resolveSubagentTranscriptTool,
  setInferenceStatusForThread,
  setPendingApprovalForThread,
  setPendingPlanReviewForThread,
  setStreamingAssistantForThread,
  setTaskBoardForThread,
  setToolTimelineForThread,
  setWorkflowProposalForThread,
  streamDeltaReceived,
  subagentAwaitingUser,
  subagentDone,
  subagentIterationStarted,
  subagentSpawned,
  subagentToolCallReceived,
  subagentToolResultReceived,
  toolArgsDeltaReceived,
  toolCallReceived,
  toolResultReceived,
  upsertArtifactFailedForThread,
  upsertArtifactInProgressForThread,
  upsertArtifactReadyForThread,
  type WorkflowProposal,
} from '../store/chatRuntimeSlice';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import {
  clearActiveExternalForThread,
  disclosureFromEvent,
  hydratePrivacyMode,
  pushDisclosureForThread,
} from '../store/privacySlice';
import { selectSocketStatus } from '../store/socketSelectors';
import {
  addInferenceResponse,
  addMessageLocal,
  clearThreadInferenceActive,
  createNewThread,
  generateThreadTitleIfNeeded,
  setActiveThread,
  setSelectedThread,
} from '../store/threadSlice';
import { IS_PROD } from '../utils/config';

const logChatRuntime = debug('openhuman:chat-runtime');
const USER_FACING_AGENT_ERROR_MESSAGE =
  'Something went wrong. Please try again.\nThis error has been reported. You can also report it on Discord.\n<openhuman-link path="community/discord-report">Report on Discord</openhuman-link>';

const SEGMENT_DELIVERY_TTL_MS = 5 * 60 * 1000;
const MAX_SEGMENT_DELIVERIES = 100;

type SegmentDelivery = { segments: Map<number, string>; createdAt: number; lastSeenAt: number };

type ThreadSliceState = ReturnType<typeof store.getState>['thread'];

/**
 * Whether a thread should be treated as occupied for proactive delivery — a
 * thread is only a valid target when it is provably "fresh" (holds no
 * messages). We check the in-memory message cache and the persisted
 * `messageCount` snapshot, so a thread that already has a conversation
 * server-side (but whose messages aren't loaded into the cache yet) is still
 * treated as occupied and never reused.
 *
 * Crucially, *unknown* thread metadata counts as occupied: when only a
 * rehydrated `selectedThreadId` is present (e.g. before `loadThreads`
 * resolves, or after caches were reset while selection was preserved),
 * `state.threads.find` returns `undefined`. Treating that as fresh would let
 * a proactive event append into a conversation we simply haven't loaded yet.
 * We fail closed and open a new thread instead. See #3713.
 */
function threadHasMessages(state: ThreadSliceState, threadId: string): boolean {
  const cached = state.messagesByThreadId[threadId];
  if (cached && cached.length > 0) return true;
  if (threadId === state.selectedThreadId && state.messages.length > 0) return true;
  const thread = state.threads.find(t => t.id === threadId);
  // Unknown metadata → fail closed (occupied) rather than risk interrupting an
  // unloaded conversation.
  if (!thread) return true;
  return thread.messageCount > 0;
}

function rtLog(message: string, fields?: Record<string, string | number | null | undefined>) {
  if (IS_PROD) return;
  if (fields && Object.keys(fields).length > 0) {
    const parts = Object.entries(fields)
      .filter(([, v]) => v !== undefined && v !== '' && v !== null)
      .map(([k, v]) => `${k}=${v}`);
    logChatRuntime('[chat-runtime] %s %s', message, parts.join(' '));
  } else {
    logChatRuntime('[chat-runtime] %s', message);
  }
}

function segmentDeliveryKey(threadId: string, requestId?: string | null): string {
  return `${threadId}:${requestId ?? 'none'}`;
}

function pruneSegmentDeliveries(deliveries: Map<string, SegmentDelivery>, now = Date.now()) {
  for (const [key, delivery] of deliveries) {
    if (now - delivery.createdAt > SEGMENT_DELIVERY_TTL_MS) {
      deliveries.delete(key);
    }
  }

  while (deliveries.size > MAX_SEGMENT_DELIVERIES) {
    let oldestKey: string | undefined;
    let oldestLastSeenAt = Number.POSITIVE_INFINITY;
    for (const [key, delivery] of deliveries) {
      if (delivery.lastSeenAt < oldestLastSeenAt) {
        oldestKey = key;
        oldestLastSeenAt = delivery.lastSeenAt;
      }
    }
    if (!oldestKey) break;
    deliveries.delete(oldestKey);
  }
}

function getOrCreateSegmentDelivery(
  deliveries: Map<string, SegmentDelivery>,
  key: string,
  now = Date.now()
): SegmentDelivery {
  pruneSegmentDeliveries(deliveries, now);
  const existing = deliveries.get(key);
  if (existing) {
    existing.lastSeenAt = now;
    return existing;
  }
  const delivery = { segments: new Map<number, string>(), createdAt: now, lastSeenAt: now };
  deliveries.set(key, delivery);
  pruneSegmentDeliveries(deliveries, now);
  return delivery;
}

function takeSegmentDelivery(
  deliveries: Map<string, SegmentDelivery>,
  key: string,
  now = Date.now()
): SegmentDelivery | undefined {
  pruneSegmentDeliveries(deliveries, now);
  const delivery = deliveries.get(key);
  deliveries.delete(key);
  return delivery;
}

function deleteSegmentDelivery(deliveries: Map<string, SegmentDelivery>, key: string) {
  pruneSegmentDeliveries(deliveries);
  deliveries.delete(key);
}

// Delivery is complete iff every expected segment_index arrived. Do NOT also
// compare reconstructed segments against event.full_response — the server
// trims each segment and normalises joiners during segmentation
// (presentation.rs::segment_for_delivery), while full_response keeps the raw
// LLM text. A byte-equality check therefore fails on virtually every
// multi-segment turn and triggers the reconciliation path, producing a
// duplicate assistant message.
function hasCompleteSegmentDelivery(
  event: ChatDoneEvent,
  delivery: SegmentDelivery | undefined
): boolean {
  const expected = event.segment_total ?? 0;
  if (expected <= 0 || !delivery) return false;
  if (delivery.segments.size < expected) return false;
  for (let i = 0; i < expected; i += 1) {
    if (!delivery.segments.has(i)) return false;
  }
  return true;
}

function chatDoneExtraMetadata(event: ChatDoneEvent): Record<string, unknown> | undefined {
  // Stamp the producing turn's request id so the final answer can be grouped
  // with its per-turn process trail (Phase 4 anchoring, Option B — see the
  // companion plan). `citations` is merged in when present.
  const meta: Record<string, unknown> = {};
  if (event.citations?.length) meta.citations = event.citations;
  if (event.request_id) meta.requestId = event.request_id;
  return Object.keys(meta).length > 0 ? meta : undefined;
}

/**
 * Map a `chat_done` event's holistic usage onto the `recordChatTurnUsage`
 * payload. Prefers the structured `usage` object (tokens + cost + context window
 * + per-sub-agent breakdown); falls back to the deprecated flat token fields for
 * any older core that still emits them.
 */
function chatTurnUsagePayload(event: ChatDoneEvent): {
  inputTokens: number;
  outputTokens: number;
  cachedTokens?: number;
  costUsd?: number;
  contextWindow?: number;
  threadId?: string;
  subAgents?: Array<{
    agentId: string;
    inputTokens: number;
    outputTokens: number;
    costUsd: number;
  }>;
} {
  const u = event.usage;
  if (u) {
    return {
      inputTokens: u.input_tokens,
      outputTokens: u.output_tokens,
      cachedTokens: u.cached_input_tokens,
      costUsd: u.cost_usd,
      contextWindow: u.context_window,
      threadId: event.thread_id,
      subAgents: (u.subagents ?? []).map(s => ({
        agentId: s.agent_id,
        inputTokens: s.input_tokens,
        outputTokens: s.output_tokens,
        costUsd: s.cost_usd,
      })),
    };
  }
  return {
    inputTokens: event.total_input_tokens ?? 0,
    outputTokens: event.total_output_tokens ?? 0,
    threadId: event.thread_id,
  };
}

/**
 * Parses a completed `propose_workflow` tool call's JSON `output` into a
 * `WorkflowProposal` for `WorkflowProposalCard` (issue B4 — agent-first
 * Workflow authoring). The tool's `execute()`
 * (`src/openhuman/flows/tools.rs`) returns
 * `{ type: "workflow_proposal", name, graph, require_approval, summary }` as
 * its `ToolResult` body; this maps that wire shape onto the store's camelCase
 * `WorkflowProposal`. Returns `null` for anything that fails to parse or
 * doesn't match the expected shape — defensive, since a malformed proposal
 * must never crash the chat runtime, it should just silently not render a
 * card.
 */
/**
 * Tool names whose successful `output` carries a `workflow_proposal` payload.
 * `propose_workflow` (first draft) and `revise_workflow` (iterative refine)
 * both return the identical wire shape (see `src/openhuman/flows/builder_tools.rs`),
 * so the runtime surfaces a `WorkflowProposalCard` from either. These run inside
 * the `workflow_builder` specialist — reached either as the main agent's own
 * tool or, in the Flows copilot / prompt-bar flow, as a delegated subagent
 * (`build_workflow`) — so BOTH `onToolResult` and `onSubagentToolResult` funnel
 * through {@link maybeParseWorkflowProposalTool}.
 */
const WORKFLOW_PROPOSAL_TOOLS = new Set(['propose_workflow', 'revise_workflow']);

/**
 * If a completed tool result is a successful workflow-builder proposal
 * (`propose_workflow`/`revise_workflow`), parse it. Returns `null` for anything
 * else so callers can cheaply gate on it. Keyed by the tool NAME + success, not
 * by agent, so a proposal surfaces whether the tool ran in the main agent or in
 * the delegated `workflow_builder` worker.
 */
function maybeParseWorkflowProposalTool(
  toolName: string,
  success: boolean,
  output: string | undefined
): WorkflowProposal | null {
  if (!success || !WORKFLOW_PROPOSAL_TOOLS.has(toolName) || !output) return null;
  return parseWorkflowProposal(output);
}

function parseWorkflowProposal(output: string): WorkflowProposal | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(output);
  } catch {
    return null;
  }
  if (!parsed || typeof parsed !== 'object') return null;
  const obj = parsed as Record<string, unknown>;
  if (obj.type !== 'workflow_proposal') return null;
  if (typeof obj.name !== 'string' || obj.graph == null) return null;

  const summary = (obj.summary ?? {}) as Record<string, unknown>;
  const rawSteps = Array.isArray(summary.steps) ? summary.steps : [];
  const steps = rawSteps
    .filter((s): s is Record<string, unknown> => !!s && typeof s === 'object')
    .map(s => ({
      kind: typeof s.kind === 'string' ? s.kind : 'unknown',
      name: typeof s.name === 'string' ? s.name : '',
      config_hint: typeof s.config_hint === 'string' ? s.config_hint : undefined,
    }));

  return {
    name: obj.name,
    graph: obj.graph,
    // The Rust tool defaults `require_approval` to `true` when the caller
    // omits it, so treat anything other than an explicit `false` as `true`
    // here too — keeps the client's fallback in lockstep with the server's.
    requireApproval: obj.require_approval !== false,
    summary: { trigger: typeof summary.trigger === 'string' ? summary.trigger : '', steps },
  };
}

const ChatRuntimeProvider = ({ children }: { children: React.ReactNode }) => {
  const dispatch = useAppDispatch();
  const { refetch: refetchSnapshot } = useRefetchSnapshotOnTurnEnd();
  const socketStatus = useAppSelector(selectSocketStatus);
  const toolTimelineByThread = useAppSelector(state => state.chatRuntime.toolTimelineByThread);
  const inferenceStatusByThread = useAppSelector(
    state => state.chatRuntime.inferenceStatusByThread
  );
  const streamingAssistantByThread = useAppSelector(
    state => state.chatRuntime.streamingAssistantByThread
  );

  const seenChatEventsRef = useRef<Map<string, number>>(new Map());
  const segmentDeliveriesRef = useRef<Map<string, SegmentDelivery>>(new Map());
  const proactiveThreadCreationPromiseRef = useRef<Promise<string | null> | null>(null);
  const proactiveDispatchQueueRef = useRef<Promise<void>>(Promise.resolve());
  const toolTimelineRef = useRef(toolTimelineByThread);
  const inferenceStatusRef = useRef(inferenceStatusByThread);
  const streamingAssistantRef = useRef(streamingAssistantByThread);
  // Measures wall-clock of each turn's tool chain against the 60s target
  // (#4273, AC3). Single instance for the provider's lifetime; observability
  // only — it never gates or cancels a turn.
  const skillLatencyRef = useRef(createSkillToolChainLatencyTracker());

  // Hydrate the current Privacy Mode once on mount so the persistent status
  // pill can show the posture immediately (#4437 / S3). Failures resolve to
  // null inside the thunk — the pill degrades gracefully.
  useEffect(() => {
    void dispatch(hydratePrivacyMode());
  }, [dispatch]);

  useEffect(() => {
    toolTimelineRef.current = toolTimelineByThread;
  }, [toolTimelineByThread]);

  useEffect(() => {
    inferenceStatusRef.current = inferenceStatusByThread;
  }, [inferenceStatusByThread]);

  useEffect(() => {
    streamingAssistantRef.current = streamingAssistantByThread;
  }, [streamingAssistantByThread]);

  const markChatEventSeen = (
    key: string,
    meta?: { threadId?: string; requestId?: string }
  ): boolean => {
    const now = Date.now();
    const cache = seenChatEventsRef.current;
    const ttlMs = 10 * 60_000;
    const maxEntries = 500;

    if (cache.has(key)) {
      rtLog('dedupe_drop', {
        key: key.length > 160 ? `${key.slice(0, 160)}…` : key,
        thread: meta?.threadId,
        request: meta?.requestId,
      });
      return false;
    }
    cache.set(key, now);

    for (const [existingKey, timestamp] of cache) {
      if (now - timestamp > ttlMs) {
        cache.delete(existingKey);
      }
    }

    while (cache.size > maxEntries) {
      const oldest = cache.keys().next().value;
      if (!oldest) break;
      cache.delete(oldest);
    }
    return true;
  };

  const proactiveMessageDigest = (input: string): string => {
    // Small non-cryptographic digest to keep dedupe keys bounded.
    let hash = 2166136261;
    for (let i = 0; i < input.length; i += 1) {
      hash ^= input.charCodeAt(i);
      hash = Math.imul(hash, 16777619);
    }
    return (hash >>> 0).toString(36);
  };

  const resolveVisibleThreadForProactive = useCallback(
    async (incomingThreadId: string): Promise<string | null> => {
      if (!incomingThreadId.startsWith('proactive:')) {
        return incomingThreadId;
      }

      const state = store.getState().thread;
      // Reuse an existing thread for proactive delivery ONLY when it is
      // fresh (no messages). Injecting a morning brief / subconscious
      // update into a thread that already holds a conversation interrupts
      // the active chat flow (#3713). Candidate priority is selected >
      // first thread; if the candidate already has messages we fall
      // through and open a dedicated new thread instead. An in-flight
      // inference thread always has at least the user's message, so it is
      // never considered fresh — that is why `activeThreadIds` is no
      // longer used as a target here.
      const candidateThreadId = state.selectedThreadId ?? state.threads[0]?.id ?? null;
      if (candidateThreadId && !threadHasMessages(state, candidateThreadId)) {
        return candidateThreadId;
      }

      if (proactiveThreadCreationPromiseRef.current) {
        return proactiveThreadCreationPromiseRef.current;
      }

      const createPromise: Promise<string | null> = (async () => {
        try {
          const newThread = await dispatch(createNewThread()).unwrap();
          dispatch(setSelectedThread(newThread.id));
          return newThread.id;
        } catch (error) {
          rtLog('proactive_thread_create_failed', {
            err: error instanceof Error ? error.message : String(error),
          });
          return null;
        } finally {
          proactiveThreadCreationPromiseRef.current = null;
        }
      })();
      proactiveThreadCreationPromiseRef.current = createPromise;

      try {
        return await createPromise;
      } finally {
        // no-op: cleared in createPromise.finally
      }
    },
    [dispatch]
  );

  useEffect(() => {
    if (socketStatus !== 'connected') return;

    // When a turn ends, any follow-ups the user queued behind it are about to be
    // dispatched by the backend as fresh turns. Nothing else persists their
    // prompt — the web channel never writes user messages; the composer does
    // (`addMessageLocal` → `appendMessage`) — so append them to the transcript
    // now. Doing it here (after this turn's assistant reply was appended, before
    // `endInferenceTurn` clears the pills) keeps the append-log order correct:
    // user → assistant → queued follow-up. Without this the queued prompts are
    // lost on reload and the dispatched answer has no visible user message.
    const flushQueuedFollowups = async (threadId: string) => {
      const queued = store.getState().chatRuntime.queuedFollowupsByThread[threadId] ?? [];
      // Persist sequentially so the queued prompts land in the append-log in the
      // order the user queued them (concurrent dispatches would race), and
      // surface failures instead of dropping them silently. The stored message
      // carries the original content + attachment metadata, so the follow-up
      // persists identically to an interactive send.
      for (const item of queued) {
        try {
          await dispatch(addMessageLocal({ threadId, message: item.message })).unwrap();
        } catch (error) {
          rtLog('flush_followup_append_failed', {
            thread: threadId,
            message: item.message.id,
            error: error instanceof Error ? error.message : String(error),
          });
        }
      }
    };

    const finishChatDoneTurn = async (event: ChatDoneEvent, path: string) => {
      rtLog('refresh_usage_counter', {
        thread: event.thread_id,
        request: event.request_id,
        reason: 'chat_done',
      });
      requestUsageRefresh();
      rtLog('snapshot_refetch_queued', {
        thread: event.thread_id,
        request: event.request_id,
        reason: 'chat_done',
        path,
      });
      refetchSnapshot();
      // Persist queued follow-ups (in order, after this turn's assistant reply)
      // and only then clear the queue + lifecycle.
      await flushQueuedFollowups(event.thread_id);
      dispatch(endInferenceTurn({ threadId: event.thread_id }));
      dispatch(clearThreadInferenceActive(event.thread_id));
    };

    rtLog('subscribe_chat_events', { socket: socketStatus });
    const cleanup = subscribeChatEvents({
      onInferenceStart: (event: ChatInferenceStartEvent) => {
        rtLog('inference_start', { thread: event.thread_id, request: event.request_id });
        // Fresh turn: drop the previous turn's live processing transcript so a
        // new turn's narration/steps don't append onto the old one.
        dispatch(clearProcessingForThread({ threadId: event.thread_id }));
        dispatch(markInferenceTurnStreaming({ threadId: event.thread_id }));
        dispatch(
          setInferenceStatusForThread({
            threadId: event.thread_id,
            status: { phase: 'thinking', iteration: 0, maxIterations: 0 },
          })
        );
      },
      onInferenceHeartbeat: (event: ChatInferenceHeartbeatEvent) => {
        // #4270: liveness beat — bump the per-thread counter so the
        // Conversations silence timer rearms even when the turn is in a long
        // prefill / buffered-reasoning phase that emits no other progress.
        rtLog('inference_heartbeat', { thread: event.thread_id, request: event.request_id });
        // A parallel (forked) turn streams into its own lane and must NOT keep
        // the thread's primary silence timer alive — otherwise a sibling branch
        // would mask a stalled primary turn. Mirror the text/thinking-delta
        // routing: ignore heartbeats owned by a parallel request.
        if (store.getState().chatRuntime.parallelRequestThreads[event.request_id] !== undefined) {
          return;
        }
        dispatch(bumpInferenceHeartbeatForThread({ threadId: event.thread_id }));
      },
      onIterationStart: (event: ChatIterationStartEvent) => {
        const prev = inferenceStatusRef.current[event.thread_id];
        rtLog('iteration_start', {
          thread: event.thread_id,
          request: event.request_id,
          iteration: event.round,
        });
        dispatch(
          setInferenceStatusForThread({
            threadId: event.thread_id,
            status: {
              phase: 'thinking',
              iteration: event.round,
              maxIterations: prev?.maxIterations ?? 0,
            },
          })
        );
      },
      onToolCall: (event: ChatToolCallEvent) => {
        const prev = store.getState().chatRuntime.inferenceStatusByThread[event.thread_id];
        dispatch(
          setInferenceStatusForThread({
            threadId: event.thread_id,
            status: {
              ...(prev ?? { iteration: event.round, maxIterations: 0 }),
              phase: 'tool_use',
              activeTool: event.tool_name,
            },
          })
        );

        const eventKey = `tool_call:${event.thread_id}:${event.request_id ?? 'none'}:${event.round}:${event.tool_name}:${event.tool_call_id ?? ''}`;
        if (
          !markChatEventSeen(eventKey, { threadId: event.thread_id, requestId: event.request_id })
        )
          return;

        // Start (or extend) the tool-chain latency window for this turn (#4273).
        // Key by thread+request (same scheme as segment delivery) so parallel /
        // forked turns that share a thread_id keep independent chains (#4288).
        skillLatencyRef.current.noteToolCall(segmentDeliveryKey(event.thread_id, event.request_id));

        // Merge + processing-pointer are now a single reducer (Phase 3) — no
        // getState()/full-array rebuild in the provider.
        dispatch(
          toolCallReceived({
            threadId: event.thread_id,
            round: event.round,
            toolName: event.tool_name,
            toolCallId: event.tool_call_id,
            displayLabel: event.tool_display_label,
            displayDetail: event.tool_display_detail,
          })
        );
      },
      onToolResult: (event: ChatToolResultEvent) => {
        const eventKey = `tool_result:${event.thread_id}:${event.request_id ?? 'none'}:${event.round}:${event.tool_name}:${event.success}:${event.tool_call_id ?? ''}`;
        if (
          !markChatEventSeen(eventKey, { threadId: event.thread_id, requestId: event.request_id })
        )
          return;

        // Settle the matching row in the reducer (Phase 3) — no getState() /
        // full-array rebuild. A no-op when no row matches.
        dispatch(
          toolResultReceived({
            threadId: event.thread_id,
            round: event.round,
            toolName: event.tool_name,
            toolCallId: event.tool_call_id,
            success: event.success,
            output: event.output,
            failure: event.failure,
          })
        );

        // Agent-first Workflow authoring (issue B4): a completed
        // `propose_workflow` call carries a `workflow_proposal` JSON payload
        // in `output` — surface it as a `WorkflowProposalCard` above the
        // composer. The tool only validates; only the card's "Save & enable"
        // action ever calls `flows_create`, so this dispatch alone can never
        // create a flow.
        const mainProposal = maybeParseWorkflowProposalTool(
          event.tool_name,
          event.success,
          event.output
        );
        if (mainProposal) {
          rtLog('workflow proposal parsed (main agent)', {
            thread: event.thread_id,
            tool: event.tool_name,
            name: mainProposal.name,
          });
          dispatch(
            setWorkflowProposalForThread({ threadId: event.thread_id, proposal: mainProposal })
          );
        }

        const current = store.getState().chatRuntime.inferenceStatusByThread[event.thread_id];
        if (!current) return;
        dispatch(
          setInferenceStatusForThread({
            threadId: event.thread_id,
            status: { ...current, phase: 'thinking', activeTool: undefined },
          })
        );
      },
      onSubagentSpawned: event => {
        const prev = store.getState().chatRuntime.inferenceStatusByThread[event.thread_id];
        dispatch(
          setInferenceStatusForThread({
            threadId: event.thread_id,
            status: {
              ...(prev ?? { iteration: event.round, maxIterations: 0 }),
              phase: 'subagent',
              activeSubagent: event.tool_name,
            },
          })
        );

        // Collapse the parent spawn/delegate row into the subagent row (one
        // entry per delegation) — merge now lives in the reducer (Phase 3).
        dispatch(
          subagentSpawned({
            threadId: event.thread_id,
            round: event.round,
            rowId: `${event.thread_id}:subagent:${event.skill_id}:${event.tool_name}`,
            taskId: event.skill_id,
            agentId: event.tool_name,
            displayName: event.subagent?.display_name,
            workerThreadId: event.subagent?.worker_thread_id,
            mode: event.subagent?.mode,
            dedicatedThread: event.subagent?.dedicated_thread,
          })
        );
      },
      onSubagentAwaitingUser: (event: ChatSubagentDoneEvent) => {
        dispatch(
          subagentAwaitingUser({
            threadId: event.thread_id,
            rowId: `${event.thread_id}:subagent:${event.skill_id}:${event.tool_name}`,
          })
        );
      },
      onSubagentDone: (event: ChatSubagentDoneEvent) => {
        // Worktree isolation metadata (#3376) — present only for workers that ran
        // with `isolation = "worktree"`; drives the inline worktree row's
        // open/diff/remove affordances. Undefined fields leave the row untouched.
        dispatch(
          subagentDone({
            threadId: event.thread_id,
            rowId: `${event.thread_id}:subagent:${event.skill_id}:${event.tool_name}`,
            success: event.success,
            iterations: event.subagent?.iterations,
            elapsedMs: event.subagent?.elapsed_ms,
            outputChars: event.subagent?.output_chars,
            worktreePath: event.subagent?.worktree_path,
            changedFiles: event.subagent?.changed_files,
            isDirty: event.subagent?.dirty_status,
          })
        );

        const current = store.getState().chatRuntime.inferenceStatusByThread[event.thread_id];
        if (!current) return;
        dispatch(
          setInferenceStatusForThread({
            threadId: event.thread_id,
            status: { ...current, phase: 'thinking', activeSubagent: undefined },
          })
        );
      },
      onSubagentIterationStart: event => {
        const taskId = event.subagent?.task_id ?? event.skill_id;
        const agentId = event.subagent?.agent_id ?? event.tool_name;
        dispatch(
          subagentIterationStarted({
            threadId: event.thread_id,
            rowId: `${event.thread_id}:subagent:${taskId}:${agentId}`,
            childIteration: event.subagent?.child_iteration,
            childMaxIterations: event.subagent?.child_max_iterations,
          })
        );
      },
      onSubagentToolCall: event => {
        const taskId = event.subagent?.task_id ?? event.skill_id;
        const agentId = event.subagent?.agent_id;
        if (!agentId) return;
        const rowId = `${event.thread_id}:subagent:${taskId}:${agentId}`;
        // Reducer owns the toolCalls upsert (dedup on call_id) — no getState().
        dispatch(
          subagentToolCallReceived({
            threadId: event.thread_id,
            rowId,
            callId: event.tool_call_id,
            toolName: event.tool_name,
            iteration: event.subagent?.child_iteration,
            args: event.args,
            displayName: event.tool_display_label,
            detail: event.tool_display_detail,
          })
        );
        // Mirror the call into the ordered transcript so the drawer renders it
        // right after the text that triggered it (self-guarded / self-deduped).
        dispatch(
          recordSubagentTranscriptTool({
            threadId: event.thread_id,
            rowId,
            callId: event.tool_call_id,
            toolName: event.tool_name,
            iteration: event.subagent?.child_iteration,
            args: event.args,
            displayName: event.tool_display_label,
            detail: event.tool_display_detail,
          })
        );
      },
      onSubagentToolResult: event => {
        // Phase 5c: the Flows prompt bar / canvas copilot route to the
        // `workflow_builder` specialist via delegation (`build_workflow`), so a
        // `propose_workflow`/`revise_workflow` proposal is produced INSIDE the
        // delegated worker and arrives here (not on `onToolResult`). This
        // extraction must run BEFORE the timeline-entry guards below: under
        // the workflow_builder subagent's heavy event volume, the progress
        // channel (bounded, `try_send`) can drop earlier events, so the
        // timeline row for this call may never have been created — gating
        // proposal extraction on finding that row silently drops the
        // proposal and the Accept/Reject card never renders (bug). The
        // extraction only needs `tool_name`/`success`/`output`, all present
        // directly on the event, with no timeline dependency. Surface it on
        // the PARENT thread (`event.thread_id`, which the progress bridge
        // always stamps with the parent request's thread, not the child's)
        // so the same `WorkflowProposalCard` the direct-tool path uses
        // renders it. Still validate-only — the card's explicit Save is the
        // sole persistence gate.
        const subagentProposal = maybeParseWorkflowProposalTool(
          event.tool_name,
          event.success,
          event.output
        );
        if (subagentProposal) {
          rtLog('workflow proposal parsed (delegated worker)', {
            thread: event.thread_id,
            tool: event.tool_name,
            name: subagentProposal.name,
          });
          dispatch(
            setWorkflowProposalForThread({ threadId: event.thread_id, proposal: subagentProposal })
          );
        }

        const taskId = event.subagent?.task_id ?? event.skill_id;
        const agentId = event.subagent?.agent_id;
        if (!agentId) return;
        const rowId = `${event.thread_id}:subagent:${taskId}:${agentId}`;
        // Reducer owns the nested toolCall settle (no-op if the call is absent).
        dispatch(
          subagentToolResultReceived({
            threadId: event.thread_id,
            rowId,
            callId: event.tool_call_id,
            success: event.success,
            elapsedMs: event.subagent?.elapsed_ms,
            outputChars: event.subagent?.output_chars,
            result: event.output,
            failure: event.failure,
          })
        );
        dispatch(
          resolveSubagentTranscriptTool({
            threadId: event.thread_id,
            rowId,
            callId: event.tool_call_id,
            success: event.success,
            elapsedMs: event.subagent?.elapsed_ms,
            outputChars: event.subagent?.output_chars,
            result: event.output,
            failure: event.success ? undefined : parseToolFailure(event.failure),
          })
        );
      },
      onSubagentTextDelta: (event: ChatSubagentTextDeltaEvent) => {
        const taskId = event.subagent?.task_id;
        const agentId = event.subagent?.agent_id;
        if (!taskId || !agentId || !event.delta) return;
        dispatch(
          appendSubagentStreamDelta({
            threadId: event.thread_id,
            rowId: `${event.thread_id}:subagent:${taskId}:${agentId}`,
            kind: 'text',
            delta: event.delta,
            iteration: event.subagent?.child_iteration,
          })
        );
      },
      onSubagentThinkingDelta: (event: ChatSubagentThinkingDeltaEvent) => {
        const taskId = event.subagent?.task_id;
        const agentId = event.subagent?.agent_id;
        if (!taskId || !agentId || !event.delta) return;
        dispatch(
          appendSubagentStreamDelta({
            threadId: event.thread_id,
            rowId: `${event.thread_id}:subagent:${taskId}:${agentId}`,
            kind: 'thinking',
            delta: event.delta,
            iteration: event.subagent?.child_iteration,
          })
        );
      },
      onSegment: (event: ChatSegmentEvent) => {
        const eventKey = `segment:${event.thread_id}:${event.request_id}:${event.segment_index}`;
        if (
          !markChatEventSeen(eventKey, { threadId: event.thread_id, requestId: event.request_id })
        )
          return;
        const content = segmentText(event);
        const deliveryKey = segmentDeliveryKey(event.thread_id, event.request_id);
        const delivery = getOrCreateSegmentDelivery(segmentDeliveriesRef.current, deliveryKey);
        delivery.segments.set(event.segment_index, content);
        void dispatch(
          addInferenceResponse({
            content,
            threadId: event.thread_id,
            // Stamp the producing turn's request id so the timeline projection
            // can group this answer with its per-turn process trail (Phase 4
            // anchoring, Option B — see the companion plan). `citations` is
            // merged in when present.
            extraMetadata: {
              ...(event.citations?.length ? { citations: event.citations } : {}),
              ...(event.request_id ? { requestId: event.request_id } : {}),
            },
          })
        );
      },
      onInterim: (event: ChatInterimEvent) => {
        // One interim per round — `round` is a stable per-turn dedup key that
        // survives socket reconnect/replay (a re-delivered frame must not
        // append the narration bubble twice).
        const eventKey = `interim:${event.thread_id}:${event.request_id}:${event.round}`;
        if (
          !markChatEventSeen(eventKey, { threadId: event.thread_id, requestId: event.request_id })
        )
          return;
        const content = event.full_response?.trim() ?? '';
        if (!content) return;
        // Persist this round's leading narration as its own interleaved bubble,
        // stamped with the producing turn's request id (Phase 4 anchoring).
        // `isInterim: true` marks this as between-tool narration rather than a
        // turn's terminal answer — the main chat still renders it as a bubble
        // (unchanged), but callers that only want the terminal turn (e.g. the
        // Flows copilot's `displayMessages`, see `useWorkflowBuilderChat`) can
        // filter it out.
        rtLog('interim_narration_tagged', {
          thread: event.thread_id,
          request: event.request_id,
          round: event.round,
        });
        void dispatch(
          addInferenceResponse({
            content,
            threadId: event.thread_id,
            extraMetadata: {
              isInterim: true,
              ...(event.request_id ? { requestId: event.request_id } : {}),
            },
          })
        );
        // The narration has now become a bubble, so drop it from the live
        // streaming preview (which accumulates across the whole turn under one
        // request_id) — otherwise the same text lingers in the preview tail and
        // reads as a duplicate for the full duration of the tool call. Reset
        // synchronously so the next round's deltas start from an empty buffer.
        const cr = store.getState().chatRuntime;
        const existing = cr.streamingAssistantByThread[event.thread_id];
        if (existing && existing.requestId === event.request_id) {
          dispatch(
            setStreamingAssistantForThread({
              threadId: event.thread_id,
              streaming: {
                requestId: existing.requestId,
                content: '',
                thinking: existing.thinking,
              },
            })
          );
        }
      },
      onTextDelta: event => {
        // Parallel-vs-primary routing + processing transcript now live in the
        // reducer (Phase 3) — no getState() in the provider.
        dispatch(
          streamDeltaReceived({
            threadId: event.thread_id,
            requestId: event.request_id,
            round: event.round,
            delta: event.delta,
            channel: 'content',
          })
        );
      },
      onThinkingDelta: event => {
        dispatch(
          streamDeltaReceived({
            threadId: event.thread_id,
            requestId: event.request_id,
            round: event.round,
            delta: event.delta,
            channel: 'thinking',
          })
        );
      },
      onToolArgsDelta: event => {
        // Match + append + decorate now live in the reducer (Phase 3).
        dispatch(
          toolArgsDeltaReceived({
            threadId: event.thread_id,
            round: event.round,
            delta: event.delta,
            toolName: event.tool_name,
            toolCallId: event.tool_call_id,
          })
        );
      },
      onTaskBoardUpdated: (event: ChatTaskBoardUpdatedEvent) => {
        if (!event.task_board) return;
        dispatch(setTaskBoardForThread({ threadId: event.thread_id, board: event.task_board }));
      },
      onProactiveMessage: (event: ProactiveMessageEvent) => {
        const messageDigest = proactiveMessageDigest(event.full_response ?? '');
        const eventKey = `proactive:${event.thread_id}:${event.request_id ?? 'none'}:${messageDigest}`;
        if (
          !markChatEventSeen(eventKey, { threadId: event.thread_id, requestId: event.request_id })
        )
          return;

        proactiveDispatchQueueRef.current = proactiveDispatchQueueRef.current.then(async () => {
          try {
            const targetThreadId = await resolveVisibleThreadForProactive(event.thread_id);
            if (!targetThreadId) return;
            rtLog('proactive_message', {
              from: event.thread_id,
              to: targetThreadId,
              request: event.request_id,
            });
            await dispatch(
              addInferenceResponse({
                content: event.full_response,
                threadId: targetThreadId,
                // Stamp the producing turn's request id when present (Phase 4
                // anchoring); proactive events may omit it, in which case the
                // message falls back to the legacy single-anchor turn.
                extraMetadata: event.request_id ? { requestId: event.request_id } : undefined,
              })
            );
          } catch (error) {
            rtLog('proactive_dispatch_failed', {
              from: event.thread_id,
              request: event.request_id,
              error: error instanceof Error ? error.message : String(error),
            });
          }
        });
      },
      onArtifactPending: event => {
        rtLog('artifact_pending', {
          thread: event.thread_id,
          artifact_id: event.artifact_id,
          kind: event.kind,
        });
        dispatch(
          upsertArtifactInProgressForThread({
            threadId: event.thread_id,
            artifactId: event.artifact_id,
            kind: event.kind,
            title: event.title,
          })
        );
      },
      onArtifactReady: event => {
        rtLog('artifact_ready', {
          thread: event.thread_id,
          artifact_id: event.artifact_id,
          kind: event.kind,
          size_bytes: event.size_bytes,
        });
        dispatch(
          upsertArtifactReadyForThread({
            threadId: event.thread_id,
            artifactId: event.artifact_id,
            kind: event.kind,
            title: event.title,
            path: event.path,
            sizeBytes: event.size_bytes,
          })
        );
      },
      onArtifactFailed: event => {
        // Defence-in-depth: producer is expected to pre-truncate the
        // reason, but cap again here so a leaky producer cannot dump
        // unbounded provider stderr into client telemetry.
        rtLog('artifact_failed', {
          thread: event.thread_id,
          artifact_id: event.artifact_id,
          kind: event.kind,
          error: event.error.slice(0, 80),
        });
        dispatch(
          upsertArtifactFailedForThread({
            threadId: event.thread_id,
            artifactId: event.artifact_id,
            kind: event.kind,
            title: event.title,
            error: event.error,
          })
        );
      },
      onExternalTransferPending: (event: ExternalTransferPendingEvent) => {
        // #4437 / S3 — DISCLOSURE ONLY. Project the egress descriptor onto the
        // thread's privacy ledger so the in-chat card + status pill can render
        // what/where/why. No approve/deny here (that's S4 #4438).
        rtLog('external_transfer_pending', {
          thread: event.thread_id,
          provider: event.provider_slug,
          service: event.service,
          reason: event.reason,
          kinds: event.data_kinds.length,
          external: String(event.is_external),
        });
        dispatch(
          pushDisclosureForThread({
            threadId: event.thread_id,
            disclosure: disclosureFromEvent(event),
          })
        );
      },
      onApprovalRequest: (event: ChatApprovalRequestEvent) => {
        rtLog('approval_request', {
          thread: event.thread_id,
          request: event.request_id,
          tool: event.tool_name,
        });
        // Pull the exact command/target out of the redacted args for display:
        // shell → command, file write/edit → path, network → url.
        const a = event.args ?? {};
        const firstString = (v: unknown): string | undefined =>
          typeof v === 'string' && v.length > 0 ? v : undefined;
        const command =
          firstString(a.command) ??
          firstString(a.path) ??
          firstString(a.url) ??
          firstString(a.target);
        // `composio_connect` carries the toolkit slug so the inline connect
        // card (#3993) knows which integration to authorize.
        const toolkit = firstString(a.toolkit);
        dispatch(
          setPendingApprovalForThread({
            threadId: event.thread_id,
            approval: {
              requestId: event.request_id,
              toolName: event.tool_name,
              message: event.message,
              command,
              toolkit,
            },
          })
        );
      },
      onPlanReviewRequest: (event: ChatPlanReviewRequestEvent) => {
        rtLog('plan_review_request', { thread: event.thread_id, request: event.request_id });
        const steps = Array.isArray(event.args?.steps)
          ? event.args.steps.filter((s): s is string => typeof s === 'string')
          : [];
        dispatch(
          setPendingPlanReviewForThread({
            threadId: event.thread_id,
            review: { requestId: event.request_id, summary: event.message, steps },
          })
        );
      },
      onDone: event => {
        const eventKey = `done:${event.thread_id}:${event.request_id ?? 'none'}`;
        if (
          !markChatEventSeen(eventKey, { threadId: event.thread_id, requestId: event.request_id })
        )
          return;

        rtLog('chat_done', {
          thread: event.thread_id,
          request: event.request_id,
          segments: event.segment_total,
          input_tokens: event.total_input_tokens,
          output_tokens: event.total_output_tokens,
        });

        // Close the tool-chain latency window and surface overruns of the 60s
        // target (#4273, AC3). Observability only — never blocks the turn.
        const latency = skillLatencyRef.current.finishChain(
          segmentDeliveryKey(event.thread_id, event.request_id),
          { ok: true }
        );
        if (latency) {
          rtLog('skill_tool_chain_latency', {
            thread: event.thread_id,
            request: event.request_id,
            elapsed_ms: latency.elapsedMs,
            tools: latency.toolCount,
            within_target: latency.withinTarget ? 'true' : 'false',
          });
          if (!latency.withinTarget) {
            console.warn(
              `[skill-latency] tool chain on thread ${event.thread_id} took ${latency.elapsedMs}ms ` +
                `across ${latency.toolCount} tool(s) — exceeds the ${SKILL_TOOL_CHAIN_TARGET_MS}ms target`
            );
          }
        }

        // Parallel (forked) turn: resolve only its own lane. The primary turn's
        // stream / status / lifecycle / active marker may still be running, so
        // we must NOT clear them here. Segmented parallel turns already
        // persisted via `onSegment` (keyed by thread+request); a single-bubble
        // parallel turn persists its full response now.
        if (
          event.request_id !== undefined &&
          store.getState().chatRuntime.parallelRequestThreads[event.request_id] !== undefined
        ) {
          const parallelRequestId = event.request_id;
          dispatch(recordChatTurnUsage(chatTurnUsagePayload(event)));
          if (!event.segment_total && event.full_response.length > 0) {
            void (async () => {
              try {
                await dispatch(
                  addInferenceResponse({
                    content: event.full_response,
                    threadId: event.thread_id,
                    extraMetadata: chatDoneExtraMetadata(event),
                  })
                ).unwrap();
                void dispatch(
                  generateThreadTitleIfNeeded({
                    threadId: event.thread_id,
                    assistantMessage: event.full_response,
                  })
                );
              } catch (error) {
                rtLog('parallel_chat_done_append_failed', {
                  thread: event.thread_id,
                  request: event.request_id,
                  error: error instanceof Error ? error.message : String(error),
                });
              }
            })();
          }
          dispatch(clearParallelRequest({ requestId: parallelRequestId }));
          requestUsageRefresh();
          return;
        }

        const deliveryKey = segmentDeliveryKey(event.thread_id, event.request_id);
        const segmentDelivery = takeSegmentDelivery(segmentDeliveriesRef.current, deliveryKey);
        const completeSegmentDelivery = hasCompleteSegmentDelivery(event, segmentDelivery);

        dispatch(recordChatTurnUsage(chatTurnUsagePayload(event)));
        dispatch(clearInferenceStatusForThread({ threadId: event.thread_id }));
        dispatch(clearStreamingAssistantForThread({ threadId: event.thread_id }));
        dispatch(clearPendingApprovalForThread({ threadId: event.thread_id }));
        dispatch(clearPendingPlanReviewForThread({ threadId: event.thread_id }));
        // Turn boundary: this turn's external transfers are done, so return the
        // privacy pill to on-device. The (un-dismissed) disclosure ledger stays
        // for the in-chat card's history — only the live flag clears (#4437).
        dispatch(clearActiveExternalForThread({ threadId: event.thread_id }));

        const existing = store.getState().chatRuntime.toolTimelineByThread[event.thread_id] ?? [];
        if (existing.length > 0) {
          const entries = existing.map(entry =>
            entry.status === 'running' ? { ...entry, status: 'success' as const } : entry
          );
          dispatch(setToolTimelineForThread({ threadId: event.thread_id, entries }));
        }
        if (!event.segment_total) {
          void (async () => {
            try {
              await dispatch(
                addInferenceResponse({
                  content: event.full_response,
                  threadId: event.thread_id,
                  extraMetadata: chatDoneExtraMetadata(event),
                })
              ).unwrap();
              void dispatch(
                generateThreadTitleIfNeeded({
                  threadId: event.thread_id,
                  assistantMessage: event.full_response,
                })
              );
            } catch (error) {
              rtLog('chat_done_append_failed', {
                thread: event.thread_id,
                request: event.request_id,
                error: error instanceof Error ? error.message : String(error),
              });
            }
            await finishChatDoneTurn(event, 'proactive');
          })();
          return;
        }

        if (!completeSegmentDelivery && event.full_response.length > 0) {
          rtLog('chat_done_segment_reconcile', {
            thread: event.thread_id,
            request: event.request_id,
            expected: event.segment_total,
            received: segmentDelivery?.segments.size ?? 0,
            full_len: event.full_response.length,
          });
          void (async () => {
            try {
              await dispatch(
                addInferenceResponse({
                  content: event.full_response,
                  threadId: event.thread_id,
                  extraMetadata: chatDoneExtraMetadata(event),
                })
              ).unwrap();
              void dispatch(
                generateThreadTitleIfNeeded({
                  threadId: event.thread_id,
                  assistantMessage: event.full_response,
                })
              );
            } catch (error) {
              rtLog('chat_done_reconcile_append_failed', {
                thread: event.thread_id,
                request: event.request_id,
                error: error instanceof Error ? error.message : String(error),
              });
            }
            await finishChatDoneTurn(event, 'segment_reconcile');
          })();
          return;
        }

        void dispatch(
          generateThreadTitleIfNeeded({
            threadId: event.thread_id,
            assistantMessage: event.full_response,
          })
        );
        void finishChatDoneTurn(event, 'ordinary');
      },
      onError: event => {
        const eventKey = `error:${event.thread_id}:${event.request_id ?? 'none'}:${event.error_type}`;
        if (
          !markChatEventSeen(eventKey, { threadId: event.thread_id, requestId: event.request_id })
        )
          return;

        rtLog('chat_error', {
          thread: event.thread_id,
          request: event.request_id,
          err: event.error_type,
        });

        // A failed turn still closes its latency window so the chain timer never
        // leaks into a later turn on the same thread (#4273, AC3).
        const errLatency = skillLatencyRef.current.finishChain(
          segmentDeliveryKey(event.thread_id, event.request_id),
          { ok: false }
        );
        if (errLatency) {
          rtLog('skill_tool_chain_latency', {
            thread: event.thread_id,
            request: event.request_id,
            elapsed_ms: errLatency.elapsedMs,
            tools: errLatency.toolCount,
            within_target: errLatency.withinTarget ? 'true' : 'false',
            ok: 'false',
          });
        }

        // #3931: surface expected, user-actionable provider/billing states
        // (insufficient BYO credits, managed-budget exhaustion) in the shell's
        // dedicated error panel — in ADDITION to the inline chat message below.
        // Additive + defensive: no-op for non-actionable errors, never throws.
        if (event.error_type !== 'cancelled') {
          ingestRuntimeErrorSignal(dispatch, {
            message: event.message,
            errorType: event.error_type,
            scope: 'chat',
            sourceDomain: 'chat',
          });
        }

        // Parallel (forked) turn error: resolve only its lane, leaving the
        // primary turn untouched. Surface a non-cancellation error as a message
        // so the failed branch is visible.
        if (
          event.request_id !== undefined &&
          store.getState().chatRuntime.parallelRequestThreads[event.request_id] !== undefined
        ) {
          deleteSegmentDelivery(
            segmentDeliveriesRef.current,
            segmentDeliveryKey(event.thread_id, event.request_id)
          );
          if (event.error_type !== 'cancelled') {
            const errorContent = event.message || USER_FACING_AGENT_ERROR_MESSAGE;
            void dispatch(
              addInferenceResponse({ content: errorContent, threadId: event.thread_id })
            );
            requestUsageRefresh();
          }
          dispatch(clearParallelRequest({ requestId: event.request_id }));
          return;
        }

        deleteSegmentDelivery(
          segmentDeliveriesRef.current,
          segmentDeliveryKey(event.thread_id, event.request_id)
        );
        dispatch(clearInferenceStatusForThread({ threadId: event.thread_id }));
        dispatch(clearStreamingAssistantForThread({ threadId: event.thread_id }));
        dispatch(clearPendingApprovalForThread({ threadId: event.thread_id }));
        dispatch(clearPendingPlanReviewForThread({ threadId: event.thread_id }));
        // Turn boundary (error path): clear the live external-transfer flag so
        // the privacy pill resets to on-device, mirroring the done path (#4437).
        dispatch(clearActiveExternalForThread({ threadId: event.thread_id }));

        const existing = store.getState().chatRuntime.toolTimelineByThread[event.thread_id] ?? [];
        if (existing.length > 0) {
          const entries = existing.map(entry =>
            entry.status === 'running' ? { ...entry, status: 'error' as const } : entry
          );
          dispatch(setToolTimelineForThread({ threadId: event.thread_id, entries }));
        }

        if (event.error_type !== 'cancelled') {
          const currentState = store.getState();
          const threadMessages = currentState.thread.messagesByThreadId[event.thread_id] ?? [];
          const lastMsg = threadMessages[threadMessages.length - 1];
          // Every error_type — including the generic 'inference' fallback — carries a
          // user-facing `message` produced by classify_inference_error() in web_errors.rs.
          // For 'inference' that message is the friendly summary PLUS the real, sanitized
          // upstream provider error appended as a `> quote` block (secret-scrubbed and
          // length-capped server-side via with_provider_detail()/sanitize_api_error()), so
          // surfacing it tells the user *why* the turn failed instead of a blanket apology.
          // The hardcoded constant is only a last-resort fallback for an empty/missing message.
          const errorContent = event.message || USER_FACING_AGENT_ERROR_MESSAGE;
          if (!(lastMsg?.sender === 'agent' && lastMsg?.content === errorContent)) {
            void dispatch(
              addInferenceResponse({ content: errorContent, threadId: event.thread_id })
            );
          }

          rtLog('refresh_usage_counter', {
            thread: event.thread_id,
            request: event.request_id,
            reason: 'chat_error',
          });
          requestUsageRefresh();
        }

        // The backend drains + dispatches queued follow-ups even when the turn
        // errored, so flush them to the transcript here too (otherwise their
        // prompts are lost). Mirrors the done path (sequential internally).
        void flushQueuedFollowups(event.thread_id);
        dispatch(endInferenceTurn({ threadId: event.thread_id }));
        dispatch(clearThreadInferenceActive(event.thread_id));
      },
    });

    return () => {
      rtLog('unsubscribe_chat_events');
      cleanup();
    };
  }, [dispatch, resolveVisibleThreadForProactive, socketStatus, refetchSnapshot]);

  // Socket-disconnect reconciliation.
  //
  // `activeThreadId` and the per-thread inference lifecycle are only ever
  // cleared by `chat_done` / `chat_error` events. If the socket drops
  // mid-turn (Windows sleep/wake, network change, VPN flap) those events
  // fire on the dead session and never reach us, so the composer stays
  // disabled until the 2-minute silence timer expires — users perceive
  // this as being "locked out" of typing.
  //
  // When the socket leaves the `connected` state, treat any in-flight
  // turn on the previous session as unrecoverable: clear the live
  // inference status, end the lifecycle row, and release `activeThreadId`
  // so the composer is immediately typeable again. Streaming assistant
  // text is preserved so the partial reply stays visible.
  useEffect(() => {
    if (socketStatus === 'connected') return;
    const state = store.getState();
    const lifecycles = state.chatRuntime.inferenceTurnLifecycleByThread;
    const threadIds = Object.keys(lifecycles);
    const activeThreadIds = Object.keys(state.thread.activeThreadIds);
    if (threadIds.length === 0 && activeThreadIds.length === 0) return;
    // Abandon any in-flight tool-chain latency windows: a disconnect tears down
    // these turns without an onDone/onError, so without this the next tool call
    // on a reused thread would attribute stale elapsed/tool counts (#4288).
    skillLatencyRef.current.reset();
    rtLog('socket_disconnect_reconcile', {
      socket: socketStatus,
      inFlight: threadIds.length,
      active: activeThreadIds.length,
    });
    for (const threadId of threadIds) {
      dispatch(clearInferenceStatusForThread({ threadId }));
      // Clear any parked approval/plan-review too: a disconnect before
      // onDone/onError would otherwise leave the card stuck for a turn that
      // can't complete.
      dispatch(clearPendingApprovalForThread({ threadId }));
      dispatch(clearPendingPlanReviewForThread({ threadId }));
      // A disconnect tears the turn down without a done/error event, so clear
      // the live external-transfer flag here too — otherwise the privacy pill
      // would stay off-device for a turn that can never complete (#4437).
      dispatch(clearActiveExternalForThread({ threadId }));
      dispatch(endInferenceTurn({ threadId }));
    }
    // A disconnect kills every in-flight turn on the dead session, so clear all
    // active markers (setActiveThread(null) clears the whole set).
    if (activeThreadIds.length > 0) {
      dispatch(setActiveThread(null));
    }
  }, [socketStatus, dispatch]);

  return <>{children}</>;
};

export default ChatRuntimeProvider;
