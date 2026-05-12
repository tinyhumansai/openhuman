import debug from 'debug';
import { useCallback, useEffect, useRef } from 'react';

import { requestUsageRefresh } from '../hooks/usageRefresh';
import { useRefetchSnapshotOnTurnEnd } from '../hooks/useRefetchSnapshotOnTurnEnd';
import {
  type ChatDoneEvent,
  type ChatInferenceStartEvent,
  type ChatIterationStartEvent,
  type ChatSegmentEvent,
  type ChatSubagentDoneEvent,
  type ChatToolCallEvent,
  type ChatToolResultEvent,
  type ProactiveMessageEvent,
  segmentText,
  subscribeChatEvents,
} from '../services/chatService';
import { store } from '../store';
import {
  clearInferenceStatusForThread,
  clearStreamingAssistantForThread,
  endInferenceTurn,
  markInferenceTurnStreaming,
  recordChatTurnUsage,
  setInferenceStatusForThread,
  setStreamingAssistantForThread,
  setToolTimelineForThread,
  type StreamingAssistantState,
  type ToolTimelineEntry,
  type ToolTimelineEntryStatus,
} from '../store/chatRuntimeSlice';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import { selectSocketStatus } from '../store/socketSelectors';
import {
  addInferenceResponse,
  createNewThread,
  generateThreadTitleIfNeeded,
  setActiveThread,
  setSelectedThread,
} from '../store/threadSlice';
import { IS_PROD } from '../utils/config';
import { formatTimelineEntry, promptFromArgsBuffer } from '../utils/toolTimelineFormatting';

const logChatRuntime = debug('openhuman:chat-runtime');
const USER_FACING_AGENT_ERROR_MESSAGE =
  'Something went wrong. Please try again.\nThis error has been reported. You can also report it on Discord.\n<openhuman-link path="community/discord">Report on Discord</openhuman-link>';

const SEGMENT_DELIVERY_TTL_MS = 5 * 60 * 1000;
const MAX_SEGMENT_DELIVERIES = 100;

type SegmentDelivery = { segments: Map<number, string>; createdAt: number; lastSeenAt: number };

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

function hasCompleteSegmentDelivery(
  event: ChatDoneEvent,
  delivery: SegmentDelivery | undefined
): boolean {
  const expected = event.segment_total ?? 0;
  if (expected <= 0 || !delivery) return false;
  if (delivery.segments.size < expected) return false;

  let reconstructed = '';
  for (let i = 0; i < expected; i += 1) {
    const segment = delivery.segments.get(i);
    if (segment === undefined) return false;
    reconstructed += segment;
  }
  return reconstructed === event.full_response;
}

function chatDoneExtraMetadata(event: ChatDoneEvent): Record<string, unknown> | undefined {
  return event.citations?.length ? { citations: event.citations } : undefined;
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
      // Resolution priority: selected > active (in-flight inference) > welcome
      // (onboarding lockdown) > first thread in list. `activeThreadId` tracks
      // the currently running inference thread — during single-threaded onboarding
      // this will typically be the welcome thread itself, so the ordering is safe.
      const targetFromState =
        state.selectedThreadId ??
        state.activeThreadId ??
        state.welcomeThreadId ??
        state.threads[0]?.id ??
        null;
      if (targetFromState) {
        return targetFromState;
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
      return { ...entry, displayName: formatted.title, detail: formatted.detail };
    };

    const finishChatDoneTurn = (event: ChatDoneEvent, path: string) => {
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
      dispatch(endInferenceTurn({ threadId: event.thread_id }));
      dispatch(setActiveThread(null));
    };

    const findPendingDelegationContext = (
      entries: ToolTimelineEntry[],
      round: number
    ): { sourceToolName?: string; prompt?: string } => {
      for (let i = entries.length - 1; i >= 0; i -= 1) {
        const entry = entries[i];
        if (entry.status !== 'running' || entry.round !== round) continue;
        if (entry.name === 'spawn_subagent' || entry.name.startsWith('delegate_')) {
          return {
            sourceToolName: entry.name,
            prompt: entry.detail ?? promptFromArgsBuffer(entry.argsBuffer),
          };
        }
      }
      return {};
    };

    rtLog('subscribe_chat_events', { socket: socketStatus });
    const cleanup = subscribeChatEvents({
      onInferenceStart: (event: ChatInferenceStartEvent) => {
        rtLog('inference_start', { thread: event.thread_id, request: event.request_id });
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

        let entries: ToolTimelineEntry[];
        if (existingIdx >= 0) {
          entries = [...existing];
          entries[existingIdx] = decorateEntry({
            ...entries[existingIdx],
            name: event.tool_name,
            round: event.round,
            status: 'running',
          });
        } else {
          entries = [
            ...existing,
            decorateEntry({
              id:
                event.tool_call_id ??
                `${event.thread_id}:${event.round}:${existing.length}:${event.tool_name}`,
              name: event.tool_name,
              round: event.round,
              status: 'running',
            }),
          ];
        }
        dispatch(setToolTimelineForThread({ threadId: event.thread_id, entries }));
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
        dispatch(
          setToolTimelineForThread({
            threadId: event.thread_id,
            entries: [
              ...existing,
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
                  mode: event.subagent?.mode,
                  dedicatedThread: event.subagent?.dedicated_thread,
                  toolCalls: [],
                },
              }),
            ],
          })
        );
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
              },
            ],
          },
        };
        dispatch(setToolTimelineForThread({ threadId: event.thread_id, entries: next }));
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
        };
        const next = [...existing];
        next[idx] = { ...entry, subagent: { ...entry.subagent, toolCalls: updatedCalls } };
        dispatch(setToolTimelineForThread({ threadId: event.thread_id, entries: next }));
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
      },
      onThinkingDelta: event => {
        const cr = store.getState().chatRuntime;
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

        const deliveryKey = segmentDeliveryKey(event.thread_id, event.request_id);
        const segmentDelivery = takeSegmentDelivery(segmentDeliveriesRef.current, deliveryKey);
        const completeSegmentDelivery = hasCompleteSegmentDelivery(event, segmentDelivery);

        dispatch(
          recordChatTurnUsage({
            inputTokens: event.total_input_tokens,
            outputTokens: event.total_output_tokens,
          })
        );
        dispatch(clearInferenceStatusForThread({ threadId: event.thread_id }));
        dispatch(clearStreamingAssistantForThread({ threadId: event.thread_id }));

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
            finishChatDoneTurn(event, 'proactive');
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
            finishChatDoneTurn(event, 'segment_reconcile');
          })();
          return;
        }

        void dispatch(
          generateThreadTitleIfNeeded({
            threadId: event.thread_id,
            assistantMessage: event.full_response,
          })
        );
        finishChatDoneTurn(event, 'ordinary');
      },
      onError: event => {
        const eventKey = `error:${event.thread_id}:${event.request_id ?? 'none'}:${event.error_type}:${event.message}`;
        if (
          !markChatEventSeen(eventKey, { threadId: event.thread_id, requestId: event.request_id })
        )
          return;

        rtLog('chat_error', {
          thread: event.thread_id,
          request: event.request_id,
          err: event.error_type,
        });

        deleteSegmentDelivery(
          segmentDeliveriesRef.current,
          segmentDeliveryKey(event.thread_id, event.request_id)
        );
        dispatch(clearInferenceStatusForThread({ threadId: event.thread_id }));
        dispatch(clearStreamingAssistantForThread({ threadId: event.thread_id }));

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
          if (
            !(lastMsg?.sender === 'agent' && lastMsg?.content === USER_FACING_AGENT_ERROR_MESSAGE)
          ) {
            void dispatch(
              addInferenceResponse({
                content: USER_FACING_AGENT_ERROR_MESSAGE,
                threadId: event.thread_id,
              })
            );
          }

          rtLog('refresh_usage_counter', {
            thread: event.thread_id,
            request: event.request_id,
            reason: 'chat_error',
          });
          requestUsageRefresh();
        }

        dispatch(endInferenceTurn({ threadId: event.thread_id }));
        dispatch(setActiveThread(null));
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
    const activeThreadId = state.thread.activeThreadId;
    if (threadIds.length === 0 && !activeThreadId) return;
    rtLog('socket_disconnect_reconcile', {
      socket: socketStatus,
      inFlight: threadIds.length,
      active: activeThreadId,
    });
    for (const threadId of threadIds) {
      dispatch(clearInferenceStatusForThread({ threadId }));
      dispatch(endInferenceTurn({ threadId }));
    }
    if (activeThreadId) {
      dispatch(setActiveThread(null));
    }
  }, [socketStatus, dispatch]);

  return <>{children}</>;
};

export default ChatRuntimeProvider;
