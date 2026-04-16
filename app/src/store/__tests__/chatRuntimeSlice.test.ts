import { describe, expect, it } from 'vitest';

import reducer, {
  beginInferenceTurn,
  clearInferenceStatusForThread,
  clearRuntimeForThread,
  clearStreamingAssistantForThread,
  clearToolTimelineForThread,
  endInferenceTurn,
  markInferenceTurnStreaming,
  setInferenceStatusForThread,
  setStreamingAssistantForThread,
  setToolTimelineForThread,
} from '../chatRuntimeSlice';

describe('chatRuntimeSlice', () => {
  it('stores and clears per-thread inference status', () => {
    const withStatus = reducer(
      undefined,
      setInferenceStatusForThread({
        threadId: 'thread-1',
        status: { phase: 'thinking', iteration: 1, maxIterations: 4 },
      })
    );

    expect(withStatus.inferenceStatusByThread['thread-1']).toEqual({
      phase: 'thinking',
      iteration: 1,
      maxIterations: 4,
    });

    const cleared = reducer(withStatus, clearInferenceStatusForThread({ threadId: 'thread-1' }));
    expect(cleared.inferenceStatusByThread['thread-1']).toBeUndefined();
  });

  it('stores and clears streaming assistant content by thread', () => {
    const withStreaming = reducer(
      undefined,
      setStreamingAssistantForThread({
        threadId: 'thread-1',
        streaming: { requestId: 'req-1', content: 'hello', thinking: 'thinking' },
      })
    );

    expect(withStreaming.streamingAssistantByThread['thread-1']).toEqual({
      requestId: 'req-1',
      content: 'hello',
      thinking: 'thinking',
    });

    const cleared = reducer(
      withStreaming,
      clearStreamingAssistantForThread({ threadId: 'thread-1' })
    );
    expect(cleared.streamingAssistantByThread['thread-1']).toBeUndefined();
  });

  it('stores and clears tool timeline by thread', () => {
    const withTimeline = reducer(
      undefined,
      setToolTimelineForThread({
        threadId: 'thread-1',
        entries: [
          {
            id: 'call-1',
            name: 'search',
            round: 1,
            status: 'running',
            argsBuffer: '{"q":"hello"}',
          },
        ],
      })
    );

    expect(withTimeline.toolTimelineByThread['thread-1']).toEqual([
      { id: 'call-1', name: 'search', round: 1, status: 'running', argsBuffer: '{"q":"hello"}' },
    ]);

    const cleared = reducer(withTimeline, clearToolTimelineForThread({ threadId: 'thread-1' }));
    expect(cleared.toolTimelineByThread['thread-1']).toBeUndefined();
  });

  it('tracks per-thread inference turn lifecycle', () => {
    const started = reducer(undefined, beginInferenceTurn({ threadId: 'thread-1' }));
    expect(started.inferenceTurnLifecycleByThread['thread-1']).toBe('started');

    const streaming = reducer(started, markInferenceTurnStreaming({ threadId: 'thread-1' }));
    expect(streaming.inferenceTurnLifecycleByThread['thread-1']).toBe('streaming');

    const ended = reducer(streaming, endInferenceTurn({ threadId: 'thread-1' }));
    expect(ended.inferenceTurnLifecycleByThread['thread-1']).toBeUndefined();
  });

  it('clears all runtime buckets for one thread', () => {
    const populated = reducer(
      reducer(
        reducer(
          undefined,
          setInferenceStatusForThread({
            threadId: 'thread-1',
            status: { phase: 'thinking', iteration: 1, maxIterations: 4 },
          })
        ),
        setStreamingAssistantForThread({
          threadId: 'thread-1',
          streaming: { requestId: 'req-1', content: 'hello', thinking: 'wait' },
        })
      ),
      setToolTimelineForThread({
        threadId: 'thread-1',
        entries: [{ id: 'call-1', name: 'search', round: 1, status: 'running' }],
      })
    );

    const withTurn = reducer(populated, beginInferenceTurn({ threadId: 'thread-1' }));
    const cleared = reducer(withTurn, clearRuntimeForThread({ threadId: 'thread-1' }));
    expect(cleared.inferenceStatusByThread['thread-1']).toBeUndefined();
    expect(cleared.streamingAssistantByThread['thread-1']).toBeUndefined();
    expect(cleared.toolTimelineByThread['thread-1']).toBeUndefined();
    expect(cleared.inferenceTurnLifecycleByThread['thread-1']).toBeUndefined();
  });
});
