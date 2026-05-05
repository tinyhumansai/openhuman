import { describe, expect, it } from 'vitest';

import type { PersistedTurnState } from '../../types/turnState';
import reducer, {
  beginInferenceTurn,
  clearInferenceStatusForThread,
  clearRuntimeForThread,
  clearStreamingAssistantForThread,
  clearToolTimelineForThread,
  endInferenceTurn,
  hydrateRuntimeFromSnapshot,
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

  it('hydrates runtime state from a persisted turn snapshot', () => {
    const snapshot: PersistedTurnState = {
      threadId: 'thread-h',
      requestId: 'req-h',
      lifecycle: 'streaming',
      iteration: 3,
      maxIterations: 25,
      phase: 'tool_use',
      activeTool: 'shell',
      streamingText: 'partial reply',
      thinking: 'reasoning…',
      toolTimeline: [
        { id: 'tc-1', name: 'shell', round: 3, status: 'running', argsBuffer: '{"cmd":"ls"}' },
      ],
      startedAt: '2026-05-04T10:00:00Z',
      updatedAt: '2026-05-04T10:00:05Z',
    };

    const next = reducer(undefined, hydrateRuntimeFromSnapshot({ snapshot }));

    expect(next.inferenceTurnLifecycleByThread['thread-h']).toBe('streaming');
    expect(next.inferenceStatusByThread['thread-h']).toEqual({
      phase: 'tool_use',
      iteration: 3,
      maxIterations: 25,
      activeTool: 'shell',
      activeSubagent: undefined,
    });
    expect(next.streamingAssistantByThread['thread-h']).toEqual({
      requestId: 'req-h',
      content: 'partial reply',
      thinking: 'reasoning…',
    });
    expect(next.toolTimelineByThread['thread-h']).toEqual([
      {
        id: 'tc-1',
        name: 'shell',
        round: 3,
        status: 'running',
        argsBuffer: '{"cmd":"ls"}',
        displayName: undefined,
        detail: undefined,
        sourceToolName: undefined,
        subagent: undefined,
      },
    ]);
  });

  it('hydrating an interrupted snapshot exposes the lifecycle for retry UI', () => {
    const snapshot: PersistedTurnState = {
      threadId: 'thread-i',
      requestId: 'req-i',
      lifecycle: 'interrupted',
      iteration: 0,
      maxIterations: 0,
      streamingText: '',
      thinking: '',
      toolTimeline: [],
      startedAt: '2026-05-04T10:00:00Z',
      updatedAt: '2026-05-04T10:00:01Z',
    };
    const next = reducer(undefined, hydrateRuntimeFromSnapshot({ snapshot }));
    expect(next.inferenceTurnLifecycleByThread['thread-i']).toBe('interrupted');
    expect(next.inferenceStatusByThread['thread-i']).toBeUndefined();
    expect(next.streamingAssistantByThread['thread-i']).toBeUndefined();
    expect(next.toolTimelineByThread['thread-i']).toEqual([]);
  });

  it('interrupted snapshot must NOT resurrect inferenceStatus / streamingAssistant from stale fields', () => {
    // Defensive: an interrupted snapshot can carry the iteration /
    // streaming buffer that was active at the moment the previous
    // process died. Hydrating those into the live-progress buckets
    // would render a fake "live" inference UI for a turn nothing is
    // driving. Lifecycle alone is the truth — buckets stay clear.
    const snapshot: PersistedTurnState = {
      threadId: 'thread-stale',
      requestId: 'req-stale',
      lifecycle: 'interrupted',
      iteration: 5,
      maxIterations: 25,
      phase: 'tool_use',
      activeTool: 'shell',
      streamingText: 'half-finished reply',
      thinking: 'half-finished thought',
      toolTimeline: [{ id: 'tc-1', name: 'shell', round: 5, status: 'running' }],
      startedAt: '2026-05-04T10:00:00Z',
      updatedAt: '2026-05-04T10:00:05Z',
    };
    const next = reducer(undefined, hydrateRuntimeFromSnapshot({ snapshot }));
    expect(next.inferenceTurnLifecycleByThread['thread-stale']).toBe('interrupted');
    expect(next.inferenceStatusByThread['thread-stale']).toBeUndefined();
    expect(next.streamingAssistantByThread['thread-stale']).toBeUndefined();
    // Tool timeline IS preserved — the UI surfaces it as a frozen
    // record next to the retry banner.
    expect(next.toolTimelineByThread['thread-stale']).toHaveLength(1);
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
