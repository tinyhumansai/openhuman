import { describe, expect, it } from 'vitest';

import inferenceReducer, {
  clearInferenceRuntimeForThread,
  setInferenceStatusForThread,
  setThreadSending,
  setToolTimelineForThread,
  upsertStreamingForThread,
} from '../inferenceSlice';

describe('inferenceSlice', () => {
  it('stores and clears runtime state per thread', () => {
    const threadId = 'thread-1';
    const withSending = inferenceReducer(undefined, setThreadSending({ threadId, sending: true }));
    const withStatus = inferenceReducer(
      withSending,
      setInferenceStatusForThread({
        threadId,
        status: { phase: 'thinking', iteration: 1, maxIterations: 3 },
      })
    );
    const withTimeline = inferenceReducer(
      withStatus,
      setToolTimelineForThread({
        threadId,
        entries: [{ id: 'call-1', name: 'search', round: 1, status: 'running' }],
      })
    );
    const withStreaming = inferenceReducer(
      withTimeline,
      upsertStreamingForThread({
        threadId,
        stream: { requestId: 'req-1', content: 'Hello', thinking: 'Thinking...' },
      })
    );

    expect(withStreaming.sendingByThread[threadId]).toBe(true);
    expect(withStreaming.inferenceStatusByThread[threadId]?.phase).toBe('thinking');
    expect(withStreaming.toolTimelineByThread[threadId]?.length).toBe(1);
    expect(withStreaming.streamingAssistantByThread[threadId]?.requestId).toBe('req-1');

    const cleared = inferenceReducer(withStreaming, clearInferenceRuntimeForThread({ threadId }));
    expect(cleared.sendingByThread[threadId]).toBeUndefined();
    expect(cleared.inferenceStatusByThread[threadId]).toBeUndefined();
    expect(cleared.toolTimelineByThread[threadId]).toBeUndefined();
    expect(cleared.streamingAssistantByThread[threadId]).toBeUndefined();
  });
});
