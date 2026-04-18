import debug from 'debug';
import { useEffect, useRef } from 'react';

import {
  type ChatInferenceStartEvent,
  type ChatIterationStartEvent,
  type ChatSegmentEvent,
  type ChatSubagentDoneEvent,
  type ChatSubagentSpawnedEvent,
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
  setActiveThread,
  setSelectedThread,
} from '../store/threadSlice';
import { requestUsageRefresh } from '../hooks/usageRefresh';
import { formatTimelineEntry, promptFromArgsBuffer } from '../utils/toolTimelineFormatting';

const logChatRuntime = debug('openhuman:chat-runtime');

function rtLog(message: string, fields?: Record<string, string | number | null | undefined>) {
  if (import.meta.env.PROD) return;
  if (fields && Object.keys(fields).length > 0) {
    const parts = Object.entries(fields)
      .filter(([, v]) => v !== undefined && v !== '' && v !== null)
      .map(([k, v]) => `${k}=${v}`);
    logChatRuntime('[chat-runtime] %s %s', message, parts.join(' '));
  } else {
    logChatRuntime('[chat-runtime] %s', message);
  }
}

const ChatRuntimeProvider = ({ children }: { children: React.ReactNode }) => {
  const dispatch = useAppDispatch();
  const socketStatus = useAppSelector(selectSocketStatus);
  const toolTimelineByThread = useAppSelector(state => state.chatRuntime.toolTimelineByThread);
  const inferenceStatusByThread = useAppSelector(
    state => state.chatRuntime.inferenceStatusByThread
  );
  const streamingAssistantByThread = useAppSelector(
    state => state.chatRuntime.streamingAssistantByThread
  );

  const seenChatEventsRef = useRef<Map<string, number>>(new Map());
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

  const resolveVisibleThreadForProactive = async (
    incomingThreadId: string
  ): Promise<string | null> => {
    if (!incomingThreadId.startsWith('proactive:')) {
      return incomingThreadId;
    }

    const state = store.getState().thread;
    const targetFromState =
      state.selectedThreadId ?? state.activeThreadId ?? state.threads[0]?.id ?? null;
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
  };

  useEffect(() => {
    if (socketStatus !== 'connected') return;

    const decorateEntry = (entry: ToolTimelineEntry): ToolTimelineEntry => {
      const formatted = formatTimelineEntry(entry);
      return { ...entry, displayName: formatted.title, detail: formatted.detail };
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
      onSubagentSpawned: (event: ChatSubagentSpawnedEvent) => {
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
              }),
            ],
          })
        );
      },
      onSubagentDone: (event: ChatSubagentDoneEvent) => {
        const subagentRowId = `${event.thread_id}:subagent:${event.skill_id}:${event.tool_name}`;
        const existing = store.getState().chatRuntime.toolTimelineByThread[event.thread_id] ?? [];
        if (existing.length > 0) {
          const entries = existing.map(entry =>
            entry.id === subagentRowId && entry.status === 'running'
              ? decorateEntry({
                  ...entry,
                  status: (event.success ? 'success' : 'error') as ToolTimelineEntryStatus,
                })
              : entry
          );
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
      onSegment: (event: ChatSegmentEvent) => {
        const eventKey = `segment:${event.thread_id}:${event.request_id}:${event.segment_index}`;
        if (
          !markChatEventSeen(eventKey, { threadId: event.thread_id, requestId: event.request_id })
        )
          return;
        void dispatch(
          addInferenceResponse({ content: segmentText(event), threadId: event.thread_id })
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
        });

        dispatch(clearInferenceStatusForThread({ threadId: event.thread_id }));
        dispatch(clearStreamingAssistantForThread({ threadId: event.thread_id }));

        const existing = store.getState().chatRuntime.toolTimelineByThread[event.thread_id] ?? [];
        if (existing.length > 0) {
          const entries = existing.map(entry =>
            entry.status === 'running' ? { ...entry, status: 'success' as const } : entry
          );
          dispatch(setToolTimelineForThread({ threadId: event.thread_id, entries }));
        }

        void (async () => {
          if (!event.segment_total) {
            await dispatch(
              addInferenceResponse({ content: event.full_response, threadId: event.thread_id })
            );
          }
        })();
        rtLog('refresh_usage_counter', {
          thread: event.thread_id,
          request: event.request_id,
          reason: 'chat_done',
        });
        requestUsageRefresh();
        dispatch(endInferenceTurn({ threadId: event.thread_id }));
        dispatch(setActiveThread(null));
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
            !(
              lastMsg?.sender === 'agent' &&
              lastMsg?.content === 'Something went wrong — please try again.'
            )
          ) {
            void dispatch(
              addInferenceResponse({
                content: 'Something went wrong — please try again.',
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
  }, [dispatch, socketStatus]);

  return <>{children}</>;
};

export default ChatRuntimeProvider;
