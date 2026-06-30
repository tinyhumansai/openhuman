import debug from 'debug';
import { useCallback, useEffect, useRef } from 'react';

import { requestUsageRefresh } from '../hooks/usageRefresh';
import { useRefetchSnapshotOnTurnEnd } from '../hooks/useRefetchSnapshotOnTurnEnd';
import { ingestRuntimeErrorSignal } from '../lib/userErrors/report';
import {
  type ChatApprovalRequestEvent,
  type ChatDoneEvent,
  type ChatInferenceStartEvent,
  type ChatIterationStartEvent,
  type ChatPlanReviewRequestEvent,
  type ChatSegmentEvent,
  type ChatSubagentDoneEvent,
  type ChatSubagentTextDeltaEvent,
  type ChatSubagentThinkingDeltaEvent,
  type ChatTaskBoardUpdatedEvent,
  type ChatToolCallEvent,
  type ChatToolResultEvent,
  type ProactiveMessageEvent,
  segmentText,
  subscribeChatEvents,
} from '../services/chatService';
import { store } from '../store';
import {
  appendProcessingProse,
  appendSubagentStreamDelta,
  clearInferenceStatusForThread,
  clearParallelRequest,
  clearPendingApprovalForThread,
  clearPendingPlanReviewForThread,
  clearProcessingForThread,
  clearStreamingAssistantForThread,
  endInferenceTurn,
  markInferenceTurnStreaming,
  recordChatTurnUsage,
  recordProcessingTool,
  recordSubagentTranscriptTool,
  resolveSubagentTranscriptTool,
  setInferenceStatusForThread,
  setParallelStream,
  setPendingApprovalForThread,
  setPendingPlanReviewForThread,
  setStreamingAssistantForThread,
  setTaskBoardForThread,
  setToolTimelineForThread,
  type StreamingAssistantState,
  type ToolTimelineEntry,
  type ToolTimelineEntryStatus,
  upsertArtifactFailedForThread,
  upsertArtifactInProgressForThread,
  upsertArtifactReadyForThread,
} from '../store/chatRuntimeSlice';
import { useAppDispatch, useAppSelector } from '../store/hooks';
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
import {
  formatTimelineEntry,
  isKnownClientTool,
  promptFromArgsBuffer,
} from '../utils/toolTimelineFormatting';

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
  return event.citations?.length ? { citations: event.citations } : undefined;
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

export function findPendingDelegationContext(
  entries: ToolTimelineEntry[],
  round: number
): { sourceToolName?: string; prompt?: string; spawnEntryId?: string } {
  for (let i = entries.length - 1; i >= 0; i -= 1) {
    const entry = entries[i];
    if (entry.status !== 'running' || entry.round !== round) continue;
    if (
      ['spawn_subagent', 'spawn_async_subagent'].includes(entry.name) ||
      entry.name.startsWith('delegate_')
    ) {
      return {
        sourceToolName: entry.name,
        prompt: entry.detail ?? promptFromArgsBuffer(entry.argsBuffer),
        spawnEntryId: entry.id,
      };
    }
  }
  return {};
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

    const decorateEntry = (entry: ToolTimelineEntry): ToolTimelineEntry => {
      const formatted = formatTimelineEntry(entry);
      // The server now attaches a human label/detail for dynamic
      // Composio/MCP/integration tools the client can't know. Trust it for
      // those; for the fixed set of built-ins the client formatter labels
      // well (with args-aware detail), the client label stays authoritative.
      if (entry.displayName && !isKnownClientTool(entry.name)) {
        return {
          ...entry,
          displayName: entry.displayName,
          detail: entry.detail ?? formatted.detail,
        };
      }
      return { ...entry, displayName: formatted.title, detail: formatted.detail ?? entry.detail };
    };

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

        const existing = store.getState().chatRuntime.toolTimelineByThread[event.thread_id] ?? [];
        const existingIdx = event.tool_call_id
          ? existing.findIndex(entry => entry.id === event.tool_call_id)
          : -1;

        // Stable row id, shared with the processing-transcript tool pointer so
        // the panel can resolve the row by `callId`.
        const rowId =
          event.tool_call_id ??
          `${event.thread_id}:${event.round}:${existing.length}:${event.tool_name}`;

        let entries: ToolTimelineEntry[];
        if (existingIdx >= 0) {
          entries = [...existing];
          entries[existingIdx] = decorateEntry({
            ...entries[existingIdx],
            name: event.tool_name,
            round: event.round,
            status: 'running',
            displayName: event.tool_display_label ?? entries[existingIdx].displayName,
            detail: event.tool_display_detail ?? entries[existingIdx].detail,
          });
        } else {
          entries = [
            ...existing,
            decorateEntry({
              id: rowId,
              name: event.tool_name,
              round: event.round,
              status: 'running',
              displayName: event.tool_display_label,
              detail: event.tool_display_detail,
            }),
          ];
        }
        dispatch(setToolTimelineForThread({ threadId: event.thread_id, entries }));
        dispatch(
          recordProcessingTool({ threadId: event.thread_id, round: event.round, callId: rowId })
        );
      },
      onToolResult: (event: ChatToolResultEvent) => {
        const eventKey = `tool_result:${event.thread_id}:${event.request_id ?? 'none'}:${event.round}:${event.tool_name}:${event.success}:${event.tool_call_id ?? ''}`;
        if (
          !markChatEventSeen(eventKey, { threadId: event.thread_id, requestId: event.request_id })
        )
          return;

        const existing = store.getState().chatRuntime.toolTimelineByThread[event.thread_id] ?? [];
        if (existing.length > 0) {
          const nextEntries = [...existing];
          let changed = false;

          if (event.tool_call_id) {
            const idx = nextEntries.findIndex(entry => entry.id === event.tool_call_id);
            if (idx >= 0) {
              nextEntries[idx] = {
                ...nextEntries[idx],
                status: event.success ? 'success' : 'error',
              };
              changed = true;
            }
          }

          if (!changed) {
            for (let i = nextEntries.length - 1; i >= 0; i -= 1) {
              const entry = nextEntries[i];
              if (
                entry.status === 'running' &&
                entry.name === event.tool_name &&
                entry.round === event.round
              ) {
                nextEntries[i] = { ...entry, status: event.success ? 'success' : 'error' };
                changed = true;
                break;
              }
            }
          }

          if (changed) {
            dispatch(setToolTimelineForThread({ threadId: event.thread_id, entries: nextEntries }));
          }
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

        const existing = store.getState().chatRuntime.toolTimelineByThread[event.thread_id] ?? [];
        const pendingContext = findPendingDelegationContext(existing, event.round);
        // Collapse the parent's `spawn_subagent`/`spawn_async_subagent`/`delegate_*` tool-call row into
        // the subagent row so the timeline shows ONE entry per delegation
        // instead of "Research" (the tool call) + "Researching" (the child).
        // The tool call's prompt is carried onto the subagent as the parent's
        // delegation message, which the drawer renders as the opening turn.
        const base = pendingContext.spawnEntryId
          ? existing.filter(e => e.id !== pendingContext.spawnEntryId)
          : existing;
        dispatch(
          setToolTimelineForThread({
            threadId: event.thread_id,
            entries: [
              ...base,
              decorateEntry({
                id: `${event.thread_id}:subagent:${event.skill_id}:${event.tool_name}`,
                name: `subagent:${event.tool_name}`,
                round: event.round,
                status: 'running',
                detail: pendingContext.prompt,
                sourceToolName: pendingContext.sourceToolName,
                subagent: {
                  taskId: event.skill_id,
                  agentId: event.tool_name,
                  displayName: event.subagent?.display_name,
                  workerThreadId: event.subagent?.worker_thread_id,
                  mode: event.subagent?.mode,
                  dedicatedThread: event.subagent?.dedicated_thread,
                  prompt: pendingContext.prompt,
                  toolCalls: [],
                  transcript: [],
                },
              }),
            ],
          })
        );
      },
      onSubagentAwaitingUser: (event: ChatSubagentDoneEvent) => {
        const subagentRowId = `${event.thread_id}:subagent:${event.skill_id}:${event.tool_name}`;
        const existing = store.getState().chatRuntime.toolTimelineByThread[event.thread_id] ?? [];
        if (existing.length > 0) {
          const entries = existing.map(entry => {
            if (entry.id !== subagentRowId || entry.status !== 'running') return entry;
            return decorateEntry({
              ...entry,
              status: 'awaiting_user' as ToolTimelineEntryStatus,
              subagent: entry.subagent
                ? { ...entry.subagent, status: 'awaiting_user' }
                : entry.subagent,
            });
          });
          dispatch(setToolTimelineForThread({ threadId: event.thread_id, entries }));
        }
      },
      onSubagentDone: (event: ChatSubagentDoneEvent) => {
        const subagentRowId = `${event.thread_id}:subagent:${event.skill_id}:${event.tool_name}`;
        const existing = store.getState().chatRuntime.toolTimelineByThread[event.thread_id] ?? [];
        if (existing.length > 0) {
          const entries = existing.map(entry => {
            if (entry.id !== subagentRowId || entry.status !== 'running') return entry;
            return decorateEntry({
              ...entry,
              status: (event.success ? 'success' : 'error') as ToolTimelineEntryStatus,
              subagent: entry.subagent
                ? {
                    ...entry.subagent,
                    iterations: event.subagent?.iterations ?? entry.subagent.iterations,
                    elapsedMs: event.subagent?.elapsed_ms ?? entry.subagent.elapsedMs,
                    outputChars: event.subagent?.output_chars ?? entry.subagent.outputChars,
                    // Worktree isolation metadata (#3376) — present only for
                    // workers that ran with `isolation = "worktree"`. Drives the
                    // inline worktree row's open/diff/remove affordances.
                    worktreePath: event.subagent?.worktree_path ?? entry.subagent.worktreePath,
                    changedFiles: event.subagent?.changed_files ?? entry.subagent.changedFiles,
                    isDirty: event.subagent?.dirty_status ?? entry.subagent.isDirty,
                  }
                : entry.subagent,
            });
          });
          dispatch(setToolTimelineForThread({ threadId: event.thread_id, entries }));
        }

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
        const rowId = `${event.thread_id}:subagent:${taskId}:${agentId}`;
        const existing = store.getState().chatRuntime.toolTimelineByThread[event.thread_id] ?? [];
        const idx = existing.findIndex(entry => entry.id === rowId);
        if (idx < 0) return;
        const entry = existing[idx];
        if (!entry.subagent) return;
        const next = [...existing];
        next[idx] = {
          ...entry,
          subagent: {
            ...entry.subagent,
            childIteration: event.subagent?.child_iteration ?? entry.subagent.childIteration,
            childMaxIterations:
              event.subagent?.child_max_iterations ?? entry.subagent.childMaxIterations,
          },
        };
        dispatch(setToolTimelineForThread({ threadId: event.thread_id, entries: next }));
      },
      onSubagentToolCall: event => {
        const taskId = event.subagent?.task_id ?? event.skill_id;
        const agentId = event.subagent?.agent_id;
        if (!agentId) return;
        const rowId = `${event.thread_id}:subagent:${taskId}:${agentId}`;
        const existing = store.getState().chatRuntime.toolTimelineByThread[event.thread_id] ?? [];
        const idx = existing.findIndex(entry => entry.id === rowId);
        if (idx < 0) return;
        const entry = existing[idx];
        if (!entry.subagent) return;
        // De-dupe on call_id — the same call should not append twice if
        // the socket layer redelivers (e.g. on reconnect during a run).
        if (entry.subagent.toolCalls.some(c => c.callId === event.tool_call_id)) return;
        const next = [...existing];
        next[idx] = {
          ...entry,
          subagent: {
            ...entry.subagent,
            toolCalls: [
              ...entry.subagent.toolCalls,
              {
                callId: event.tool_call_id,
                toolName: event.tool_name,
                status: 'running',
                iteration: event.subagent?.child_iteration,
                args: event.args,
                displayName: event.tool_display_label,
                detail: event.tool_display_detail,
              },
            ],
          },
        };
        dispatch(setToolTimelineForThread({ threadId: event.thread_id, entries: next }));
        // Mirror the call into the ordered transcript so the drawer renders
        // it right after the text that triggered it (chronological view).
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
        const taskId = event.subagent?.task_id ?? event.skill_id;
        const agentId = event.subagent?.agent_id;
        if (!agentId) return;
        const rowId = `${event.thread_id}:subagent:${taskId}:${agentId}`;
        const existing = store.getState().chatRuntime.toolTimelineByThread[event.thread_id] ?? [];
        const idx = existing.findIndex(entry => entry.id === rowId);
        if (idx < 0) return;
        const entry = existing[idx];
        if (!entry.subagent) return;
        const callIdx = entry.subagent.toolCalls.findIndex(c => c.callId === event.tool_call_id);
        if (callIdx < 0) return;
        const updatedCalls = [...entry.subagent.toolCalls];
        updatedCalls[callIdx] = {
          ...updatedCalls[callIdx],
          status: event.success ? 'success' : 'error',
          elapsedMs: event.subagent?.elapsed_ms ?? updatedCalls[callIdx].elapsedMs,
          outputChars: event.subagent?.output_chars ?? updatedCalls[callIdx].outputChars,
          result: event.output ?? updatedCalls[callIdx].result,
        };
        const next = [...existing];
        next[idx] = { ...entry, subagent: { ...entry.subagent, toolCalls: updatedCalls } };
        dispatch(setToolTimelineForThread({ threadId: event.thread_id, entries: next }));
        dispatch(
          resolveSubagentTranscriptTool({
            threadId: event.thread_id,
            rowId,
            callId: event.tool_call_id,
            success: event.success,
            elapsedMs: event.subagent?.elapsed_ms,
            outputChars: event.subagent?.output_chars,
            result: event.output,
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
            extraMetadata: event.citations?.length ? { citations: event.citations } : undefined,
          })
        );
      },
      onTextDelta: event => {
        const cr = store.getState().chatRuntime;
        // A parallel (forked) turn streams into its own lane so it doesn't
        // clobber the primary turn's stream on the same thread.
        if (cr.parallelRequestThreads[event.request_id] !== undefined) {
          const prev = cr.parallelStreamsByThread[event.thread_id]?.[event.request_id];
          dispatch(
            setParallelStream({
              threadId: event.thread_id,
              streaming: {
                requestId: event.request_id,
                content: `${prev?.content ?? ''}${event.delta}`,
                thinking: prev?.thinking ?? '',
              },
            })
          );
          return;
        }
        const existing = cr.streamingAssistantByThread[event.thread_id];
        let streaming: StreamingAssistantState;
        if (existing && existing.requestId !== event.request_id) {
          streaming = { requestId: event.request_id, content: event.delta, thinking: '' };
        } else {
          streaming = {
            requestId: event.request_id,
            content: `${existing?.content ?? ''}${event.delta}`,
            thinking: existing?.thinking ?? '',
          };
        }
        dispatch(setStreamingAssistantForThread({ threadId: event.thread_id, streaming }));
        // Build the live interleaved processing transcript so a mid-turn
        // "View processing" isn't empty (the persisted one lands on settle).
        dispatch(
          appendProcessingProse({
            threadId: event.thread_id,
            kind: 'narration',
            round: event.round,
            delta: event.delta,
          })
        );
      },
      onThinkingDelta: event => {
        const cr = store.getState().chatRuntime;
        if (cr.parallelRequestThreads[event.request_id] !== undefined) {
          const prev = cr.parallelStreamsByThread[event.thread_id]?.[event.request_id];
          dispatch(
            setParallelStream({
              threadId: event.thread_id,
              streaming: {
                requestId: event.request_id,
                content: prev?.content ?? '',
                thinking: `${prev?.thinking ?? ''}${event.delta}`,
              },
            })
          );
          return;
        }
        const existing = cr.streamingAssistantByThread[event.thread_id];
        let streaming: StreamingAssistantState;
        if (existing && existing.requestId !== event.request_id) {
          streaming = { requestId: event.request_id, content: '', thinking: event.delta };
        } else {
          streaming = {
            requestId: event.request_id,
            content: existing?.content ?? '',
            thinking: `${existing?.thinking ?? ''}${event.delta}`,
          };
        }
        dispatch(setStreamingAssistantForThread({ threadId: event.thread_id, streaming }));
        dispatch(
          appendProcessingProse({
            threadId: event.thread_id,
            kind: 'thinking',
            round: event.round,
            delta: event.delta,
          })
        );
      },
      onToolArgsDelta: event => {
        const cr = store.getState().chatRuntime;
        const existing = cr.toolTimelineByThread[event.thread_id] ?? [];
        let matchIdx = -1;
        if (event.tool_call_id) {
          matchIdx = existing.findIndex(entry => entry.id === event.tool_call_id);
        }
        if (matchIdx < 0 && event.tool_name) {
          matchIdx = existing.findIndex(
            entry =>
              entry.status === 'running' &&
              entry.name === event.tool_name &&
              entry.round === event.round
          );
        }

        let entries: ToolTimelineEntry[];
        if (matchIdx >= 0) {
          entries = [...existing];
          entries[matchIdx] = decorateEntry({
            ...entries[matchIdx],
            argsBuffer: `${entries[matchIdx].argsBuffer ?? ''}${event.delta}`,
            name:
              entries[matchIdx].name.length === 0 && event.tool_name
                ? event.tool_name
                : entries[matchIdx].name,
          });
        } else {
          entries = [
            ...existing,
            decorateEntry({
              id: event.tool_call_id,
              name: event.tool_name ?? '',
              round: event.round,
              status: 'running',
              argsBuffer: event.delta,
            }),
          ];
        }
        dispatch(setToolTimelineForThread({ threadId: event.thread_id, entries }));
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
              addInferenceResponse({ content: event.full_response, threadId: targetThreadId })
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
