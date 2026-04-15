import debug from 'debug';

import { store } from '../store';
import {
  clearInferenceStatusForThread,
  clearStreamingForThread,
  setInferenceStatusForThread,
  setThreadSending,
  setToolTimelineForThread,
  upsertStreamingForThread,
} from '../store/inferenceSlice';
import { addInferenceResponse, persistReaction, setActiveThread } from '../store/threadSlice';
import {
  type ChatDoneEvent,
  type ChatErrorEvent,
  type ChatInferenceStartEvent,
  type ChatIterationStartEvent,
  type ChatSegmentEvent,
  type ChatSubagentDoneEvent,
  type ChatSubagentSpawnedEvent,
  type ChatTextDeltaEvent,
  type ChatThinkingDeltaEvent,
  type ChatToolArgsDeltaEvent,
  type ChatToolCallEvent,
  type ChatToolResultEvent,
  segmentText,
  subscribeChatEvents,
} from './chatService';

type CleanupFn = () => void;

const chatEventLog = debug('realtime:chat-event-manager');

interface PendingReaction {
  msgId: string;
  content: string;
  threadId: string;
}

let cleanupSubscription: CleanupFn | null = null;
const seenChatEvents = new Map<string, number>();
const pendingReactionByThread = new Map<string, PendingReaction>();

function markChatEventSeen(key: string): boolean {
  const now = Date.now();
  const ttlMs = 10 * 60_000;
  const maxEntries = 500;

  if (seenChatEvents.has(key)) return false;
  seenChatEvents.set(key, now);

  for (const [existingKey, timestamp] of seenChatEvents) {
    if (now - timestamp > ttlMs) {
      seenChatEvents.delete(existingKey);
    }
  }

  while (seenChatEvents.size > maxEntries) {
    const oldest = seenChatEvents.keys().next().value;
    if (!oldest) break;
    seenChatEvents.delete(oldest);
  }

  return true;
}

function applyReactionIfPending(threadId: string, emoji: string | null | undefined) {
  if (!emoji) return;
  const pending = pendingReactionByThread.get(threadId);
  if (!pending) return;
  void store.dispatch(
    persistReaction({ threadId: pending.threadId, messageId: pending.msgId, emoji })
  );
  pendingReactionByThread.delete(threadId);
}

function setInferenceStart(event: ChatInferenceStartEvent) {
  store.dispatch(
    setInferenceStatusForThread({
      threadId: event.thread_id,
      status: { phase: 'thinking', iteration: 0, maxIterations: 0 },
    })
  );
}

function setIterationStart(event: ChatIterationStartEvent) {
  const current = store.getState().inference.inferenceStatusByThread[event.thread_id];
  store.dispatch(
    setInferenceStatusForThread({
      threadId: event.thread_id,
      status: {
        phase: 'thinking',
        iteration: event.round,
        maxIterations: current?.maxIterations ?? 0,
      },
    })
  );
}

function setToolCall(event: ChatToolCallEvent) {
  const current = store.getState().inference.inferenceStatusByThread[event.thread_id];
  store.dispatch(
    setInferenceStatusForThread({
      threadId: event.thread_id,
      status: {
        ...(current ?? { iteration: event.round, maxIterations: 0 }),
        phase: 'tool_use',
        activeTool: event.tool_name,
      },
    })
  );

  const eventKey = `tool_call:${event.thread_id}:${event.request_id ?? 'none'}:${event.round}:${event.tool_name}:${event.tool_call_id ?? ''}`;
  if (!markChatEventSeen(eventKey)) return;

  const existing = store.getState().inference.toolTimelineByThread[event.thread_id] ?? [];
  const existingIdx = event.tool_call_id
    ? existing.findIndex(entry => entry.id === event.tool_call_id)
    : -1;

  if (existingIdx >= 0) {
    const merged = [...existing];
    merged[existingIdx] = {
      ...merged[existingIdx],
      name: event.tool_name,
      round: event.round,
      status: 'running',
    };
    store.dispatch(setToolTimelineForThread({ threadId: event.thread_id, entries: merged }));
    return;
  }

  store.dispatch(
    setToolTimelineForThread({
      threadId: event.thread_id,
      entries: [
        ...existing,
        {
          id:
            event.tool_call_id ??
            `${event.thread_id}:${event.round}:${existing.length}:${event.tool_name}`,
          name: event.tool_name,
          round: event.round,
          status: 'running',
        },
      ],
    })
  );
}

function setToolResult(event: ChatToolResultEvent) {
  const eventKey = `tool_result:${event.thread_id}:${event.request_id ?? 'none'}:${event.round}:${event.tool_name}:${event.success}:${event.tool_call_id ?? ''}`;
  if (!markChatEventSeen(eventKey)) return;

  const existing = store.getState().inference.toolTimelineByThread[event.thread_id] ?? [];
  if (existing.length > 0) {
    const nextEntries = [...existing];
    let changed = false;

    if (event.tool_call_id) {
      const idx = nextEntries.findIndex(entry => entry.id === event.tool_call_id);
      if (idx >= 0) {
        nextEntries[idx] = { ...nextEntries[idx], status: event.success ? 'success' : 'error' };
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
      store.dispatch(setToolTimelineForThread({ threadId: event.thread_id, entries: nextEntries }));
    }
  }

  const current = store.getState().inference.inferenceStatusByThread[event.thread_id];
  if (!current) return;
  store.dispatch(
    setInferenceStatusForThread({
      threadId: event.thread_id,
      status: { ...current, phase: 'thinking', activeTool: undefined },
    })
  );
}

function setSubagentSpawned(event: ChatSubagentSpawnedEvent) {
  const current = store.getState().inference.inferenceStatusByThread[event.thread_id];
  store.dispatch(
    setInferenceStatusForThread({
      threadId: event.thread_id,
      status: {
        ...(current ?? { iteration: event.round, maxIterations: 0 }),
        phase: 'subagent',
        activeSubagent: event.tool_name,
      },
    })
  );

  const existing = store.getState().inference.toolTimelineByThread[event.thread_id] ?? [];
  store.dispatch(
    setToolTimelineForThread({
      threadId: event.thread_id,
      entries: [
        ...existing,
        {
          id: `${event.thread_id}:subagent:${event.skill_id}:${event.tool_name}`,
          name: `🤖 ${event.tool_name}`,
          round: event.round,
          status: 'running',
        },
      ],
    })
  );
}

function setSubagentDone(event: ChatSubagentDoneEvent) {
  const existing = store.getState().inference.toolTimelineByThread[event.thread_id] ?? [];
  if (existing.length > 0) {
    store.dispatch(
      setToolTimelineForThread({
        threadId: event.thread_id,
        entries: existing.map(entry =>
          entry.name === `🤖 ${event.tool_name}` && entry.status === 'running'
            ? { ...entry, status: event.success ? 'success' : 'error' }
            : entry
        ),
      })
    );
  }

  const current = store.getState().inference.inferenceStatusByThread[event.thread_id];
  if (!current) return;
  store.dispatch(
    setInferenceStatusForThread({
      threadId: event.thread_id,
      status: { ...current, phase: 'thinking', activeSubagent: undefined },
    })
  );
}

function setSegment(event: ChatSegmentEvent) {
  const eventKey = `segment:${event.thread_id}:${event.request_id}:${event.segment_index}`;
  if (!markChatEventSeen(eventKey)) return;

  applyReactionIfPending(event.thread_id, event.reaction_emoji);
  void store.dispatch(
    addInferenceResponse({ content: segmentText(event), threadId: event.thread_id })
  );
}

function setTextDelta(event: ChatTextDeltaEvent) {
  const existing = store.getState().inference.streamingAssistantByThread[event.thread_id];
  if (existing && existing.requestId !== event.request_id) {
    store.dispatch(
      upsertStreamingForThread({
        threadId: event.thread_id,
        stream: { requestId: event.request_id, content: event.delta, thinking: '' },
      })
    );
    return;
  }

  store.dispatch(
    upsertStreamingForThread({
      threadId: event.thread_id,
      stream: {
        requestId: event.request_id,
        content: (existing?.content ?? '') + event.delta,
        thinking: existing?.thinking ?? '',
      },
    })
  );
}

function setThinkingDelta(event: ChatThinkingDeltaEvent) {
  const existing = store.getState().inference.streamingAssistantByThread[event.thread_id];
  if (existing && existing.requestId !== event.request_id) {
    store.dispatch(
      upsertStreamingForThread({
        threadId: event.thread_id,
        stream: { requestId: event.request_id, content: '', thinking: event.delta },
      })
    );
    return;
  }

  store.dispatch(
    upsertStreamingForThread({
      threadId: event.thread_id,
      stream: {
        requestId: event.request_id,
        content: existing?.content ?? '',
        thinking: (existing?.thinking ?? '') + event.delta,
      },
    })
  );
}

function setToolArgsDelta(event: ChatToolArgsDeltaEvent) {
  const existing = store.getState().inference.toolTimelineByThread[event.thread_id] ?? [];
  let matchIdx = -1;

  if (event.tool_call_id) {
    matchIdx = existing.findIndex(entry => entry.id === event.tool_call_id);
  }

  if (matchIdx < 0 && event.tool_name) {
    matchIdx = existing.findIndex(
      entry =>
        entry.status === 'running' && entry.name === event.tool_name && entry.round === event.round
    );
  }

  if (matchIdx >= 0) {
    const merged = [...existing];
    merged[matchIdx] = {
      ...merged[matchIdx],
      argsBuffer: (merged[matchIdx].argsBuffer ?? '') + event.delta,
      name:
        merged[matchIdx].name.length === 0 && event.tool_name
          ? event.tool_name
          : merged[matchIdx].name,
    };
    store.dispatch(setToolTimelineForThread({ threadId: event.thread_id, entries: merged }));
    return;
  }

  store.dispatch(
    setToolTimelineForThread({
      threadId: event.thread_id,
      entries: [
        ...existing,
        {
          id: event.tool_call_id,
          name: event.tool_name ?? '',
          round: event.round,
          status: 'running',
          argsBuffer: event.delta,
        },
      ],
    })
  );
}

function setDone(event: ChatDoneEvent) {
  const eventKey = `done:${event.thread_id}:${event.request_id ?? 'none'}`;
  if (!markChatEventSeen(eventKey)) return;

  store.dispatch(clearInferenceStatusForThread({ threadId: event.thread_id }));
  store.dispatch(clearStreamingForThread({ threadId: event.thread_id }));
  store.dispatch(setThreadSending({ threadId: event.thread_id, sending: false }));

  const existing = store.getState().inference.toolTimelineByThread[event.thread_id] ?? [];
  if (existing.length > 0) {
    store.dispatch(
      setToolTimelineForThread({
        threadId: event.thread_id,
        entries: existing.map(entry =>
          entry.status === 'running' ? { ...entry, status: 'success' as const } : entry
        ),
      })
    );
  }

  applyReactionIfPending(event.thread_id, event.reaction_emoji);
  pendingReactionByThread.delete(event.thread_id);

  if (!event.segment_total) {
    void store.dispatch(
      addInferenceResponse({ content: event.full_response, threadId: event.thread_id })
    );
  }

  store.dispatch(setActiveThread(null));
}

function setError(event: ChatErrorEvent) {
  const eventKey = `error:${event.thread_id}:${event.request_id ?? 'none'}:${event.error_type}:${event.message}`;
  if (!markChatEventSeen(eventKey)) return;

  store.dispatch(setThreadSending({ threadId: event.thread_id, sending: false }));
  store.dispatch(clearInferenceStatusForThread({ threadId: event.thread_id }));
  store.dispatch(clearStreamingForThread({ threadId: event.thread_id }));

  const existing = store.getState().inference.toolTimelineByThread[event.thread_id] ?? [];
  if (existing.length > 0) {
    store.dispatch(
      setToolTimelineForThread({
        threadId: event.thread_id,
        entries: existing.map(entry =>
          entry.status === 'running' ? { ...entry, status: 'error' as const } : entry
        ),
      })
    );
  }

  pendingReactionByThread.delete(event.thread_id);

  if (event.error_type !== 'cancelled') {
    const threadMessages = store.getState().thread.messagesByThreadId[event.thread_id] ?? [];
    const lastMsg = threadMessages[threadMessages.length - 1];
    if (
      lastMsg?.sender !== 'agent' ||
      lastMsg?.content !== 'Something went wrong — please try again.'
    ) {
      void store.dispatch(
        addInferenceResponse({
          content: 'Something went wrong — please try again.',
          threadId: event.thread_id,
        })
      );
    }
  }

  store.dispatch(setActiveThread(null));
}

export const chatEventManager = {
  init() {
    if (cleanupSubscription) return;

    chatEventLog('[chat-event-manager] init: subscribing to chat events');
    cleanupSubscription = subscribeChatEvents({
      onInferenceStart: setInferenceStart,
      onIterationStart: setIterationStart,
      onToolCall: setToolCall,
      onToolResult: setToolResult,
      onSubagentSpawned: setSubagentSpawned,
      onSubagentDone: setSubagentDone,
      onSegment: setSegment,
      onTextDelta: setTextDelta,
      onThinkingDelta: setThinkingDelta,
      onToolArgsDelta: setToolArgsDelta,
      onDone: setDone,
      onError: setError,
    });
  },
  teardown() {
    if (!cleanupSubscription) return;

    chatEventLog('[chat-event-manager] teardown: unsubscribing from chat events');
    cleanupSubscription();
    cleanupSubscription = null;
  },
  setPendingReaction(payload: PendingReaction) {
    pendingReactionByThread.set(payload.threadId, payload);
  },
  clearPendingReaction(threadId: string) {
    pendingReactionByThread.delete(threadId);
  },
};
