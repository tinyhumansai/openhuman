import { describe, expect, it } from 'vitest';

import type { PersistedTurnState } from '../../types/turnState';
import reducer, {
  beginInferenceTurn,
  bumpInferenceHeartbeatForThread,
  clearAllChatRuntime,
  clearArtifactsForThread,
  clearInferenceStatusForThread,
  clearParallelRequest,
  clearPendingApprovalForThread,
  clearRuntimeForThread,
  clearStreamingAssistantForThread,
  clearTaskBoardForThread,
  clearToolTimelineForThread,
  endInferenceTurn,
  hydrateRuntimeFromRunLedger,
  hydrateRuntimeFromSnapshot,
  markInferenceTurnStreaming,
  registerParallelRequest,
  removeArtifactForThread,
  setInferenceStatusForThread,
  setParallelStream,
  setPendingApprovalForThread,
  setStreamingAssistantForThread,
  setTaskBoardForThread,
  setToolTimelineForThread,
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

  it('bumps the per-thread inference heartbeat counter and clears it on runtime reset (#4270)', () => {
    const once = reducer(undefined, bumpInferenceHeartbeatForThread({ threadId: 'thread-1' }));
    expect(once.inferenceHeartbeatByThread['thread-1']).toBe(1);

    // Each beat advances the counter — the silence-timer rearm watches the
    // *change*, so a monotonically-different value per beat is what matters.
    const twice = reducer(once, bumpInferenceHeartbeatForThread({ threadId: 'thread-1' }));
    expect(twice.inferenceHeartbeatByThread['thread-1']).toBe(2);

    // A different thread tracks its own counter independently.
    const other = reducer(twice, bumpInferenceHeartbeatForThread({ threadId: 'thread-2' }));
    expect(other.inferenceHeartbeatByThread['thread-2']).toBe(1);
    expect(other.inferenceHeartbeatByThread['thread-1']).toBe(2);

    // Turn teardown drops the liveness counter so a stale value can't rearm a
    // fresh turn's timer.
    const cleared = reducer(other, clearRuntimeForThread({ threadId: 'thread-1' }));
    expect(cleared.inferenceHeartbeatByThread['thread-1']).toBeUndefined();
    expect(cleared.inferenceHeartbeatByThread['thread-2']).toBe(1);

    // Slice-wide reset wipes every thread's heartbeat too — no stale counter
    // survives a full chat-runtime clear (CodeRabbit #4282).
    const wiped = reducer(other, clearAllChatRuntime());
    expect(wiped.inferenceHeartbeatByThread).toEqual({});
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
            seq: 0,
            status: 'running',
            argsBuffer: '{"q":"hello"}',
          },
        ],
      })
    );

    expect(withTimeline.toolTimelineByThread['thread-1']).toEqual([
      {
        id: 'call-1',
        name: 'search',
        round: 1,
        seq: 0,
        status: 'running',
        argsBuffer: '{"q":"hello"}',
      },
    ]);

    const cleared = reducer(withTimeline, clearToolTimelineForThread({ threadId: 'thread-1' }));
    expect(cleared.toolTimelineByThread['thread-1']).toBeUndefined();
  });

  it('stores task boards by thread and hydrates them from snapshots', () => {
    const taskBoard = {
      threadId: 'thread-board',
      updatedAt: '2026-05-04T10:00:05Z',
      cards: [
        {
          id: 'task-1',
          title: 'Draft plan',
          status: 'todo' as const,
          order: 0,
          updatedAt: '2026-05-04T10:00:05Z',
        },
      ],
    };

    const withBoard = reducer(
      undefined,
      setTaskBoardForThread({ threadId: 'thread-board', board: taskBoard })
    );
    expect(withBoard.taskBoardByThread['thread-board']).toEqual(taskBoard);

    const afterClear = reducer(withBoard, clearTaskBoardForThread({ threadId: 'thread-board' }));
    expect(afterClear.taskBoardByThread['thread-board']).toBeUndefined();

    const snapshot: PersistedTurnState = {
      threadId: 'thread-h',
      requestId: 'req-h',
      lifecycle: 'streaming',
      iteration: 1,
      maxIterations: 25,
      streamingText: '',
      thinking: '',
      toolTimeline: [],
      taskBoard,
      startedAt: '2026-05-04T10:00:00Z',
      updatedAt: '2026-05-04T10:00:05Z',
    };
    const hydrated = reducer(undefined, hydrateRuntimeFromSnapshot({ snapshot }));
    expect(hydrated.taskBoardByThread['thread-h']).toEqual(taskBoard);
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
        seq: 0,
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

  it('rehydrates historical subagent rows without live streamed prose', () => {
    const snapshot: PersistedTurnState = {
      threadId: 'thread-subagent',
      requestId: 'req-subagent',
      lifecycle: 'interrupted',
      iteration: 2,
      maxIterations: 25,
      streamingText: '',
      thinking: '',
      toolTimeline: [
        {
          id: 'subagent:sub-1',
          name: 'subagent:researcher',
          round: 2,
          status: 'running',
          subagent: {
            taskId: 'sub-1',
            agentId: 'researcher',
            status: 'awaiting_user',
            mode: 'typed',
            workerThreadId: 'worker-1',
            toolCalls: [
              {
                callId: 'child-tool-1',
                toolName: 'search_web',
                status: 'success',
                iteration: 1,
                elapsedMs: 44,
                outputChars: 128,
              },
            ],
          },
        },
      ],
      startedAt: '2026-06-04T12:00:00Z',
      updatedAt: '2026-06-04T12:00:08Z',
    };

    const next = reducer(undefined, hydrateRuntimeFromSnapshot({ snapshot }));
    const row = next.toolTimelineByThread['thread-subagent'][0];

    expect(row.subagent?.status).toBe('awaiting_user');
    expect(row.subagent?.workerThreadId).toBe('worker-1');
    expect(row.subagent?.toolCalls).toHaveLength(1);
    expect(row.subagent?.transcript).toEqual([
      {
        kind: 'tool',
        iteration: 1,
        callId: 'child-tool-1',
        toolName: 'search_web',
        status: 'success',
        elapsedMs: 44,
        outputChars: 128,
      },
    ]);
  });

  it('hydrates compact historical subagent rows from durable run ledger rows', () => {
    const next = reducer(
      undefined,
      hydrateRuntimeFromRunLedger({
        threadId: 'thread-runs',
        runs: [
          {
            id: 'sub-run-1',
            kind: 'worker_thread',
            parentRunId: 'req-run-1',
            parentThreadId: 'thread-runs',
            agentId: 'researcher',
            status: 'awaiting_user',
            workerThreadId: 'worker-1',
            checkpoint: { resumeTool: 'continue_subagent' },
            summary: 'Which repository should I inspect?',
            metadata: { mode: 'typed', dedicatedThread: true, displayName: 'Researcher' },
            telemetry: {
              runId: 'sub-run-1',
              inputTokens: 0,
              outputTokens: 0,
              cachedInputTokens: 0,
              costUsd: 0,
              elapsedMs: 1200,
              toolCount: 2,
            },
            startedAt: '2026-06-04T12:00:00Z',
            updatedAt: '2026-06-04T12:00:04Z',
          },
        ],
      })
    );

    const row = next.toolTimelineByThread['thread-runs'][0];
    expect(row).toMatchObject({
      id: 'subagent:sub-run-1',
      name: 'subagent:researcher',
      status: 'awaiting_user',
      displayName: 'Researcher',
      detail: 'Which repository should I inspect?',
      sourceToolName: 'run_ledger',
    });
    expect(row.subagent).toMatchObject({
      taskId: 'sub-run-1',
      agentId: 'researcher',
      status: 'awaiting_user',
      displayName: 'Researcher',
      workerThreadId: 'worker-1',
      mode: 'typed',
      dedicatedThread: true,
      elapsedMs: 1200,
    });
    expect(row.subagent?.transcript).toEqual([]);
  });

  it('maps durable run ledger status, kind, and optional metadata into timeline rows', () => {
    const next = reducer(
      undefined,
      hydrateRuntimeFromRunLedger({
        threadId: 'thread-runs',
        runs: [
          {
            id: 'done-run',
            kind: 'subagent',
            parentThreadId: 'thread-runs',
            agentId: 'writer',
            status: 'completed',
            metadata: {},
            startedAt: '2026-06-04T12:00:00Z',
            updatedAt: '2026-06-04T12:00:04Z',
          },
          {
            id: 'failed-run',
            kind: 'workflow_child',
            parentThreadId: 'thread-runs',
            agentId: 'reviewer',
            status: 'failed',
            error: 'Tool failed',
            metadata: {},
            startedAt: '2026-06-04T12:00:05Z',
            updatedAt: '2026-06-04T12:00:07Z',
          },
          {
            id: 'pending-run',
            kind: 'team_member',
            parentThreadId: 'thread-runs',
            status: 'pending',
            metadata: {},
            startedAt: '2026-06-04T12:00:08Z',
            updatedAt: '2026-06-04T12:00:09Z',
          },
          {
            id: 'background-run',
            kind: 'background_agent',
            parentThreadId: 'thread-runs',
            status: 'running',
            metadata: {},
            startedAt: '2026-06-04T12:00:10Z',
            updatedAt: '2026-06-04T12:00:11Z',
          },
        ],
      })
    );

    const rows = next.toolTimelineByThread['thread-runs'];
    expect(rows).toHaveLength(3);
    expect(rows.map(row => row.status)).toEqual(['success', 'error', 'running']);
    expect(rows[1].detail).toBe('Tool failed');
    expect(rows[2]).toMatchObject({
      id: 'subagent:pending-run',
      name: 'subagent:agent',
      displayName: 'agent',
      detail: undefined,
    });
    expect(rows[2].subagent).toMatchObject({
      agentId: 'agent',
      workerThreadId: undefined,
      mode: undefined,
      dedicatedThread: undefined,
    });
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
        entries: [{ id: 'call-1', name: 'search', round: 1, seq: 0, status: 'running' }],
      })
    );

    const withTurn = reducer(populated, beginInferenceTurn({ threadId: 'thread-1' }));
    const cleared = reducer(withTurn, clearRuntimeForThread({ threadId: 'thread-1' }));
    expect(cleared.inferenceStatusByThread['thread-1']).toBeUndefined();
    expect(cleared.streamingAssistantByThread['thread-1']).toBeUndefined();
    expect(cleared.toolTimelineByThread['thread-1']).toBeUndefined();
    expect(cleared.taskBoardByThread['thread-1']).toBeUndefined();
    expect(cleared.inferenceTurnLifecycleByThread['thread-1']).toBeUndefined();
  });

  describe('pending approval (ApprovalGate surface)', () => {
    const approval = {
      requestId: 'req-approval-1',
      toolName: 'shell',
      message: 'Run `npm test` in the project',
    };

    it('stores and clears a pending approval per thread', () => {
      const withApproval = reducer(
        undefined,
        setPendingApprovalForThread({ threadId: 'thread-1', approval })
      );
      expect(withApproval.pendingApprovalByThread['thread-1']).toEqual(approval);

      const cleared = reducer(
        withApproval,
        clearPendingApprovalForThread({ threadId: 'thread-1' })
      );
      expect(cleared.pendingApprovalByThread['thread-1']).toBeUndefined();
    });

    it('keeps approvals isolated across threads', () => {
      const a = reducer(undefined, setPendingApprovalForThread({ threadId: 't1', approval }));
      const b = reducer(
        a,
        setPendingApprovalForThread({
          threadId: 't2',
          approval: { ...approval, requestId: 'req-2' },
        })
      );
      const clearedT1 = reducer(b, clearPendingApprovalForThread({ threadId: 't1' }));
      expect(clearedT1.pendingApprovalByThread['t1']).toBeUndefined();
      expect(clearedT1.pendingApprovalByThread['t2']?.requestId).toBe('req-2');
    });

    it('clearRuntimeForThread drops a stale parked approval', () => {
      const withApproval = reducer(
        undefined,
        setPendingApprovalForThread({ threadId: 'thread-1', approval })
      );
      const cleared = reducer(withApproval, clearRuntimeForThread({ threadId: 'thread-1' }));
      expect(cleared.pendingApprovalByThread['thread-1']).toBeUndefined();
    });

    it('clearAllChatRuntime drops all pending approvals', () => {
      const withApproval = reducer(
        undefined,
        setPendingApprovalForThread({ threadId: 'thread-1', approval })
      );
      const cleared = reducer(withApproval, clearAllChatRuntime());
      expect(cleared.pendingApprovalByThread).toEqual({});
    });
  });

  describe('removeArtifactForThread (#3024)', () => {
    it('removes a single artifact from a bucket while leaving siblings intact', () => {
      let state = reducer(
        undefined,
        upsertArtifactReadyForThread({
          threadId: 't1',
          artifactId: 'a',
          kind: 'presentation',
          title: 'A',
          path: 'artifacts/a.pptx',
          sizeBytes: 100,
        })
      );
      state = reducer(
        state,
        upsertArtifactReadyForThread({
          threadId: 't1',
          artifactId: 'b',
          kind: 'document',
          title: 'B',
          path: 'artifacts/b.pdf',
          sizeBytes: 200,
        })
      );
      const next = reducer(state, removeArtifactForThread({ threadId: 't1', artifactId: 'a' }));
      expect(next.artifactsByThread['t1']).toHaveLength(1);
      expect(next.artifactsByThread['t1'][0].artifactId).toBe('b');
    });

    it('drops the thread key entirely when the last artifact is removed', () => {
      const seeded = reducer(
        undefined,
        upsertArtifactReadyForThread({
          threadId: 't1',
          artifactId: 'a',
          kind: 'presentation',
          title: 'A',
          path: 'artifacts/a.pptx',
          sizeBytes: 100,
        })
      );
      const next = reducer(seeded, removeArtifactForThread({ threadId: 't1', artifactId: 'a' }));
      expect(next.artifactsByThread['t1']).toBeUndefined();
    });

    it('is a no-op for an unknown thread or unknown id', () => {
      const seeded = reducer(
        undefined,
        upsertArtifactReadyForThread({
          threadId: 't1',
          artifactId: 'a',
          kind: 'presentation',
          title: 'A',
          path: 'artifacts/a.pptx',
          sizeBytes: 100,
        })
      );
      const noThread = reducer(
        seeded,
        removeArtifactForThread({ threadId: 'nope', artifactId: 'a' })
      );
      expect(noThread.artifactsByThread['t1']).toHaveLength(1);

      const noId = reducer(
        seeded,
        removeArtifactForThread({ threadId: 't1', artifactId: 'missing' })
      );
      expect(noId.artifactsByThread['t1']).toHaveLength(1);
    });

    it('replaces an existing snapshot in place (status promotion in_progress → ready)', () => {
      // Covers the upsertArtifact "found at idx" branch — the snapshot
      // must update in place so the inline card flips status without
      // remounting.
      let state = reducer(
        undefined,
        upsertArtifactInProgressForThread({
          threadId: 't1',
          artifactId: 'a',
          kind: 'presentation',
          title: 'Live',
        })
      );
      expect(state.artifactsByThread['t1']).toHaveLength(1);
      expect(state.artifactsByThread['t1'][0].status).toBe('in_progress');

      state = reducer(
        state,
        upsertArtifactReadyForThread({
          threadId: 't1',
          artifactId: 'a',
          kind: 'presentation',
          title: 'Live',
          path: 'artifacts/a.pptx',
          sizeBytes: 4096,
        })
      );
      // Same artifactId — count must NOT grow; status flips in place.
      expect(state.artifactsByThread['t1']).toHaveLength(1);
      expect(state.artifactsByThread['t1'][0].status).toBe('ready');
      expect(state.artifactsByThread['t1'][0].path).toBe('artifacts/a.pptx');
      expect(state.artifactsByThread['t1'][0].sizeBytes).toBe(4096);
    });

    it('coexists with in_progress siblings without disturbing them', () => {
      let state = reducer(
        undefined,
        upsertArtifactInProgressForThread({
          threadId: 't1',
          artifactId: 'in-flight',
          kind: 'presentation',
          title: 'Live',
        })
      );
      state = reducer(
        state,
        upsertArtifactReadyForThread({
          threadId: 't1',
          artifactId: 'done',
          kind: 'presentation',
          title: 'Done',
          path: 'artifacts/done.pptx',
          sizeBytes: 1,
        })
      );
      const next = reducer(state, removeArtifactForThread({ threadId: 't1', artifactId: 'done' }));
      expect(next.artifactsByThread['t1']).toHaveLength(1);
      expect(next.artifactsByThread['t1'][0].artifactId).toBe('in-flight');
      expect(next.artifactsByThread['t1'][0].status).toBe('in_progress');
    });
  });

  describe('upsertArtifactFailedForThread (#3024)', () => {
    it('appends a new failed snapshot with the producer-supplied error', () => {
      const next = reducer(
        undefined,
        upsertArtifactFailedForThread({
          threadId: 't1',
          artifactId: 'a',
          kind: 'presentation',
          title: 'Bad Deck',
          error: 'engine failed: validation rejected slides[0]',
        })
      );
      expect(next.artifactsByThread['t1']).toHaveLength(1);
      const entry = next.artifactsByThread['t1'][0];
      expect(entry.status).toBe('failed');
      expect(entry.error).toBe('engine failed: validation rejected slides[0]');
      expect(entry.title).toBe('Bad Deck');
      expect(entry.kind).toBe('presentation');
    });

    it('promotes an in-flight snapshot to failed in place (same artifactId)', () => {
      const seeded = reducer(
        undefined,
        upsertArtifactInProgressForThread({
          threadId: 't1',
          artifactId: 'a',
          kind: 'presentation',
          title: 'Live',
        })
      );
      const next = reducer(
        seeded,
        upsertArtifactFailedForThread({
          threadId: 't1',
          artifactId: 'a',
          kind: 'presentation',
          title: 'Live',
          error: 'timeout',
        })
      );
      expect(next.artifactsByThread['t1']).toHaveLength(1);
      expect(next.artifactsByThread['t1'][0].status).toBe('failed');
      expect(next.artifactsByThread['t1'][0].error).toBe('timeout');
    });
  });

  describe('clearArtifactsForThread (#3024)', () => {
    it('drops the entire bucket for the named thread', () => {
      let state = reducer(
        undefined,
        upsertArtifactReadyForThread({
          threadId: 't1',
          artifactId: 'a',
          kind: 'presentation',
          title: 'A',
          path: 'artifacts/a.pptx',
          sizeBytes: 100,
        })
      );
      state = reducer(
        state,
        upsertArtifactReadyForThread({
          threadId: 't2',
          artifactId: 'b',
          kind: 'document',
          title: 'B',
          path: 'artifacts/b.pdf',
          sizeBytes: 200,
        })
      );
      const next = reducer(state, clearArtifactsForThread({ threadId: 't1' }));
      expect(next.artifactsByThread['t1']).toBeUndefined();
      // Sibling thread is untouched.
      expect(next.artifactsByThread['t2']).toHaveLength(1);
    });

    it('is safe to call against an unknown thread (no-op)', () => {
      const next = reducer(undefined, clearArtifactsForThread({ threadId: 'never-seen' }));
      expect(next.artifactsByThread).toEqual({});
    });
  });

  // Pins the cross-reducer contract: clearRuntimeForThread is a soft reset
  // (drops in-flight turn state, pending approvals, tool timelines, task
  // board) but *preserves* artifact ledgers so the Files panel + chat
  // ArtifactCard surfaces don't lose ready deck rows on a routine
  // turn-clear. clearAllChatRuntime is a hard reset (signout / workspace
  // switch) and *does* drop artifacts. Per graycyrus on PR #3026: the
  // kind of contract that silently regresses on a refactor without a
  // pinning test — also a CodeRabbit nit. (#3024)
  describe('clear-semantics: artifacts preserved vs cleared (#3024)', () => {
    it('clearRuntimeForThread preserves ready artifacts on the same thread', () => {
      const seeded = reducer(
        undefined,
        upsertArtifactReadyForThread({
          threadId: 't1',
          artifactId: 'a',
          kind: 'presentation',
          title: 'A',
          path: 'artifacts/a.pptx',
          sizeBytes: 100,
        })
      );
      const cleared = reducer(seeded, clearRuntimeForThread({ threadId: 't1' }));
      expect(cleared.artifactsByThread['t1']).toHaveLength(1);
      expect(cleared.artifactsByThread['t1'][0].artifactId).toBe('a');
      expect(cleared.artifactsByThread['t1'][0].status).toBe('ready');
    });

    it('clearAllChatRuntime drops every thread bucket', () => {
      let state = reducer(
        undefined,
        upsertArtifactReadyForThread({
          threadId: 't1',
          artifactId: 'a',
          kind: 'presentation',
          title: 'A',
          path: 'artifacts/a.pptx',
          sizeBytes: 100,
        })
      );
      state = reducer(
        state,
        upsertArtifactReadyForThread({
          threadId: 't2',
          artifactId: 'b',
          kind: 'document',
          title: 'B',
          path: 'artifacts/b.pdf',
          sizeBytes: 200,
        })
      );
      const cleared = reducer(state, clearAllChatRuntime());
      expect(cleared.artifactsByThread).toEqual({});
    });
  });

  describe('parallel (forked) turn lane', () => {
    it('registers a parallel request and streams into its own lane keyed by requestId', () => {
      let state = reducer(
        undefined,
        registerParallelRequest({ threadId: 't-1', requestId: 'req-a' })
      );
      expect(state.parallelRequestThreads['req-a']).toBe('t-1');

      state = reducer(
        state,
        setParallelStream({
          threadId: 't-1',
          streaming: { requestId: 'req-a', content: 'hi', thinking: '' },
        })
      );
      expect(state.parallelStreamsByThread['t-1']['req-a'].content).toBe('hi');
    });

    it('keeps two concurrent same-thread branches separate', () => {
      let state = reducer(undefined, registerParallelRequest({ threadId: 't-1', requestId: 'r1' }));
      state = reducer(state, registerParallelRequest({ threadId: 't-1', requestId: 'r2' }));
      state = reducer(
        state,
        setParallelStream({
          threadId: 't-1',
          streaming: { requestId: 'r1', content: 'one', thinking: '' },
        })
      );
      state = reducer(
        state,
        setParallelStream({
          threadId: 't-1',
          streaming: { requestId: 'r2', content: 'two', thinking: '' },
        })
      );
      expect(Object.keys(state.parallelStreamsByThread['t-1'])).toEqual(['r1', 'r2']);
      expect(state.parallelStreamsByThread['t-1']['r1'].content).toBe('one');
      expect(state.parallelStreamsByThread['t-1']['r2'].content).toBe('two');
    });

    it('clearParallelRequest removes one branch and drops the thread bucket when empty', () => {
      let state = reducer(undefined, registerParallelRequest({ threadId: 't-1', requestId: 'r1' }));
      state = reducer(state, registerParallelRequest({ threadId: 't-1', requestId: 'r2' }));
      state = reducer(
        state,
        setParallelStream({
          threadId: 't-1',
          streaming: { requestId: 'r1', content: 'one', thinking: '' },
        })
      );
      state = reducer(
        state,
        setParallelStream({
          threadId: 't-1',
          streaming: { requestId: 'r2', content: 'two', thinking: '' },
        })
      );

      state = reducer(state, clearParallelRequest({ requestId: 'r1' }));
      expect(state.parallelRequestThreads['r1']).toBeUndefined();
      expect(state.parallelStreamsByThread['t-1']['r1']).toBeUndefined();
      expect(state.parallelStreamsByThread['t-1']['r2'].content).toBe('two');

      state = reducer(state, clearParallelRequest({ requestId: 'r2' }));
      expect(state.parallelStreamsByThread['t-1']).toBeUndefined();
      expect(state.parallelRequestThreads).toEqual({});
    });

    it('clearRuntimeForThread drops the thread parallel streams and their request mappings', () => {
      let state = reducer(undefined, registerParallelRequest({ threadId: 't-1', requestId: 'r1' }));
      state = reducer(
        state,
        setParallelStream({
          threadId: 't-1',
          streaming: { requestId: 'r1', content: 'one', thinking: '' },
        })
      );
      // An unrelated thread's parallel branch must survive.
      state = reducer(state, registerParallelRequest({ threadId: 't-2', requestId: 'r9' }));
      state = reducer(
        state,
        setParallelStream({
          threadId: 't-2',
          streaming: { requestId: 'r9', content: 'keep', thinking: '' },
        })
      );

      state = reducer(state, clearRuntimeForThread({ threadId: 't-1' }));
      expect(state.parallelStreamsByThread['t-1']).toBeUndefined();
      expect(state.parallelRequestThreads['r1']).toBeUndefined();
      expect(state.parallelStreamsByThread['t-2']['r9'].content).toBe('keep');
      expect(state.parallelRequestThreads['r9']).toBe('t-2');
    });

    it('clearParallelRequest is a no-op for an unknown requestId', () => {
      const state = reducer(undefined, clearParallelRequest({ requestId: 'nope' }));
      expect(state.parallelStreamsByThread).toEqual({});
      expect(state.parallelRequestThreads).toEqual({});
    });
  });
});

describe('toolCallReceived (Phase 3 reducer-side merge)', () => {
  it('appends a new running row with a generated id and records the processing pointer', () => {
    const state = reducer(
      undefined,
      toolCallReceived({ threadId: 't1', round: 0, toolName: 'shell' })
    );
    const rows = state.toolTimelineByThread['t1'];
    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({ id: 't1:0:0:shell', name: 'shell', status: 'running' });
    // Fold-in of the processing-transcript pointer (was a second dispatch).
    expect(state.processingByThread['t1']).toEqual([
      { kind: 'toolCall', round: 0, seq: 0, callId: 't1:0:0:shell' },
    ]);
  });

  it('upserts an existing row by toolCallId instead of duplicating', () => {
    let state = reducer(
      undefined,
      toolCallReceived({ threadId: 't1', round: 0, toolName: 'shell', toolCallId: 'call-1' })
    );
    state = reducer(
      state,
      toolCallReceived({ threadId: 't1', round: 1, toolName: 'shell', toolCallId: 'call-1' })
    );
    const rows = state.toolTimelineByThread['t1'];
    expect(rows).toHaveLength(1);
    expect(rows[0].round).toBe(1);
    // The row keeps its original `seq` across the upsert — issue order is set
    // once, at first creation, and must not shift on a later update event.
    expect(rows[0].seq).toBe(0);
    // Processing pointer is recorded once for the stable callId.
    expect(state.processingByThread['t1']).toHaveLength(1);
  });

  it('assigns a monotonically increasing seq to each newly created row per thread', () => {
    let state = reducer(
      undefined,
      toolCallReceived({ threadId: 't1', round: 0, toolName: 'search', toolCallId: 'c1' })
    );
    state = reducer(
      state,
      toolCallReceived({ threadId: 't1', round: 0, toolName: 'search', toolCallId: 'c2' })
    );
    state = reducer(
      state,
      toolCallReceived({ threadId: 't1', round: 0, toolName: 'search', toolCallId: 'c3' })
    );
    const rows = state.toolTimelineByThread['t1'];
    expect(rows.map(r => r.seq)).toEqual([0, 1, 2]);
    // A different thread gets its own independent counter.
    state = reducer(
      state,
      toolCallReceived({ threadId: 't2', round: 0, toolName: 'search', toolCallId: 'd1' })
    );
    expect(state.toolTimelineByThread['t2'][0].seq).toBe(0);
  });

  it('keeps the seq from first creation when args arrive before the tool_call event', () => {
    // A `tool_args_delta` for call B can race ahead of call A's `tool_call`
    // event when the agent issues two parallel calls in one turn. The row
    // created by whichever event lands first gets seq 0; the row's `seq`
    // must not change when the (later) sibling event for the same call
    // arrives and only updates the existing row in place.
    let state = reducer(
      undefined,
      toolArgsDeltaReceived({
        threadId: 't1',
        round: 0,
        delta: '{"q":"b"}',
        toolName: 'search',
        toolCallId: 'call-b',
      })
    );
    state = reducer(
      state,
      toolCallReceived({ threadId: 't1', round: 0, toolName: 'search', toolCallId: 'call-a' })
    );
    // call-b's row was created first (seq 0) even though call-a's
    // `tool_call` event is semantically "first" in the pair — the point is
    // that whichever event creates the row locks in the seq, and a later
    // `toolCallReceived` for that same id does not reassign it.
    state = reducer(
      state,
      toolCallReceived({ threadId: 't1', round: 0, toolName: 'search', toolCallId: 'call-b' })
    );
    const rows = state.toolTimelineByThread['t1'];
    const rowB = rows.find(r => r.id === 'call-b');
    const rowA = rows.find(r => r.id === 'call-a');
    expect(rowB?.seq).toBe(0);
    expect(rowA?.seq).toBe(1);
    // Re-receiving call-b's tool_call event (args arrived first) must not
    // bump its seq to a later value.
    expect(rowB?.seq).toBe(0);
  });
});

describe('toolResultReceived (Phase 3 reducer-side merge)', () => {
  const withRunningRow = () =>
    reducer(
      undefined,
      setToolTimelineForThread({
        threadId: 't1',
        entries: [{ id: 'call-1', name: 'shell', round: 0, seq: 0, status: 'running' }],
      })
    );

  it('settles the row matched by toolCallId, attaching output', () => {
    const state = reducer(
      withRunningRow(),
      toolResultReceived({
        threadId: 't1',
        round: 0,
        toolName: 'shell',
        toolCallId: 'call-1',
        success: true,
        output: 'done',
      })
    );
    expect(state.toolTimelineByThread['t1'][0]).toMatchObject({
      status: 'success',
      result: 'done',
    });
  });

  it('falls back to the (only) running row of the same name+round when no id matches', () => {
    const state = reducer(
      withRunningRow(),
      toolResultReceived({ threadId: 't1', round: 0, toolName: 'shell', success: false })
    );
    expect(state.toolTimelineByThread['t1'][0].status).toBe('error');
  });

  it('FIFO: settles the oldest (not newest) running row of the same name+round when no id matches', () => {
    // Two parallel `get_tool_contract` calls with the same name+round and no
    // toolCallId on the result (mirrors a legacy/incomplete socket payload).
    // The fallback must settle the row that was issued FIRST (lowest seq),
    // not the most recently created one — otherwise a result for the first
    // call can incorrectly settle the second call's still-running row.
    let state = reducer(
      undefined,
      setToolTimelineForThread({
        threadId: 't1',
        entries: [
          { id: 'call-old', name: 'get_tool_contract', round: 0, seq: 0, status: 'running' },
          { id: 'call-new', name: 'get_tool_contract', round: 0, seq: 1, status: 'running' },
        ],
      })
    );
    state = reducer(
      state,
      toolResultReceived({
        threadId: 't1',
        round: 0,
        toolName: 'get_tool_contract',
        success: true,
        output: 'first result',
      })
    );
    const rows = state.toolTimelineByThread['t1'];
    const oldRow = rows.find(r => r.id === 'call-old');
    const newRow = rows.find(r => r.id === 'call-new');
    expect(oldRow?.status).toBe('success');
    expect(oldRow?.result).toBe('first result');
    // The newer call's row is untouched — still running, awaiting its own result.
    expect(newRow?.status).toBe('running');
    expect(newRow?.result).toBeUndefined();
  });

  it('is a no-op when nothing matches (mirrors the provider changed-guard)', () => {
    const before = withRunningRow();
    const after = reducer(
      before,
      toolResultReceived({ threadId: 't1', round: 9, toolName: 'other', success: true })
    );
    expect(after.toolTimelineByThread['t1']).toEqual(before.toolTimelineByThread['t1']);
  });
});

describe('streamDeltaReceived (Phase 3 reducer-side merge)', () => {
  it('appends a content delta to the primary stream and coalesces processing narration', () => {
    let state = reducer(
      undefined,
      streamDeltaReceived({
        threadId: 't1',
        requestId: 'r1',
        round: 0,
        delta: 'Hel',
        channel: 'content',
      })
    );
    state = reducer(
      state,
      streamDeltaReceived({
        threadId: 't1',
        requestId: 'r1',
        round: 0,
        delta: 'lo',
        channel: 'content',
      })
    );
    expect(state.streamingAssistantByThread['t1']).toEqual({
      requestId: 'r1',
      content: 'Hello',
      thinking: '',
    });
    // Two deltas coalesce into one narration block.
    expect(state.processingByThread['t1']).toEqual([
      { kind: 'narration', round: 0, seq: 0, text: 'Hello' },
    ]);
  });

  it('starts a fresh preview when the requestId changes (drops the prior tail)', () => {
    let state = reducer(
      undefined,
      streamDeltaReceived({
        threadId: 't1',
        requestId: 'r1',
        round: 0,
        delta: 'old',
        channel: 'content',
      })
    );
    state = reducer(
      state,
      streamDeltaReceived({
        threadId: 't1',
        requestId: 'r2',
        round: 0,
        delta: 'new',
        channel: 'thinking',
      })
    );
    expect(state.streamingAssistantByThread['t1']).toEqual({
      requestId: 'r2',
      content: '',
      thinking: 'new',
    });
  });

  it('routes a forked (parallel) turn into its own lane without touching the primary or processing', () => {
    let state = reducer(
      undefined,
      registerParallelRequest({ threadId: 't1', requestId: 'branch' })
    );
    state = reducer(
      state,
      streamDeltaReceived({
        threadId: 't1',
        requestId: 'branch',
        round: 0,
        delta: 'B',
        channel: 'content',
      })
    );
    expect(state.parallelStreamsByThread['t1']['branch']).toEqual({
      requestId: 'branch',
      content: 'B',
      thinking: '',
    });
    expect(state.streamingAssistantByThread['t1']).toBeUndefined();
    expect(state.processingByThread['t1']).toBeUndefined();
  });
});

describe('subagent event reducers (Phase 3)', () => {
  const spawn = (threadId = 't1') =>
    reducer(
      undefined,
      subagentSpawned({
        threadId,
        round: 0,
        rowId: 't1:subagent:task-1:researcher',
        taskId: 'task-1',
        agentId: 'researcher',
        displayName: 'Researcher',
      })
    );

  it('subagentSpawned collapses the parent spawn row into the subagent row', () => {
    // Seed a running parent delegate row for round 0.
    let state = reducer(
      undefined,
      setToolTimelineForThread({
        threadId: 't1',
        entries: [
          {
            id: 'spawn-1',
            name: 'spawn_subagent',
            round: 0,
            seq: 0,
            status: 'running',
            detail: 'go research',
          },
        ],
      })
    );
    state = reducer(
      state,
      subagentSpawned({
        threadId: 't1',
        round: 0,
        rowId: 't1:subagent:task-1:researcher',
        taskId: 'task-1',
        agentId: 'researcher',
      })
    );
    const rows = state.toolTimelineByThread['t1'];
    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({
      id: 't1:subagent:task-1:researcher',
      name: 'subagent:researcher',
      status: 'running',
      detail: 'go research', // carried from the collapsed spawn row's prompt
    });
    expect(rows[0].subagent).toMatchObject({ taskId: 'task-1', agentId: 'researcher' });
  });

  it('subagentToolCallReceived appends and de-dupes on callId; result settles it', () => {
    let state = spawn();
    const row = 't1:subagent:task-1:researcher';
    state = reducer(
      state,
      subagentToolCallReceived({ threadId: 't1', rowId: row, callId: 'c1', toolName: 'grep' })
    );
    // Redelivery is a no-op.
    state = reducer(
      state,
      subagentToolCallReceived({ threadId: 't1', rowId: row, callId: 'c1', toolName: 'grep' })
    );
    let sub = state.toolTimelineByThread['t1'][0].subagent!;
    expect(sub.toolCalls).toHaveLength(1);
    expect(sub.toolCalls[0].status).toBe('running');

    state = reducer(
      state,
      subagentToolResultReceived({
        threadId: 't1',
        rowId: row,
        callId: 'c1',
        success: true,
        result: 'ok',
      })
    );
    sub = state.toolTimelineByThread['t1'][0].subagent!;
    expect(sub.toolCalls[0]).toMatchObject({ status: 'success', result: 'ok' });
  });

  it('subagentDone settles the row + metadata; awaiting/iteration update in place', () => {
    let state = spawn();
    const row = 't1:subagent:task-1:researcher';
    state = reducer(
      state,
      subagentIterationStarted({
        threadId: 't1',
        rowId: row,
        childIteration: 2,
        childMaxIterations: 5,
      })
    );
    expect(state.toolTimelineByThread['t1'][0].subagent).toMatchObject({
      childIteration: 2,
      childMaxIterations: 5,
    });

    const awaiting = reducer(state, subagentAwaitingUser({ threadId: 't1', rowId: row }));
    expect(awaiting.toolTimelineByThread['t1'][0].status).toBe('awaiting_user');

    const done = reducer(
      state,
      subagentDone({ threadId: 't1', rowId: row, success: true, iterations: 3, elapsedMs: 42 })
    );
    expect(done.toolTimelineByThread['t1'][0]).toMatchObject({ status: 'success' });
    expect(done.toolTimelineByThread['t1'][0].subagent).toMatchObject({
      iterations: 3,
      elapsedMs: 42,
    });
  });

  it('settles an awaiting_user row on done (it must not stay stuck)', () => {
    let state = spawn();
    const row = 't1:subagent:task-1:researcher';
    state = reducer(state, subagentAwaitingUser({ threadId: 't1', rowId: row }));
    expect(state.toolTimelineByThread['t1'][0].status).toBe('awaiting_user');
    state = reducer(state, subagentDone({ threadId: 't1', rowId: row, success: true }));
    expect(state.toolTimelineByThread['t1'][0].status).toBe('success');
  });

  it('done is a no-op once the row is terminal (already settled)', () => {
    let state = spawn();
    const row = 't1:subagent:task-1:researcher';
    state = reducer(state, subagentDone({ threadId: 't1', rowId: row, success: true }));
    // A second done cannot re-settle a terminal row.
    const again = reducer(state, subagentDone({ threadId: 't1', rowId: row, success: false }));
    expect(again.toolTimelineByThread['t1'][0].status).toBe('success');
  });

  it('subagentSpawned is idempotent — a redelivered event does not duplicate the row', () => {
    let state = spawn();
    const before = state.toolTimelineByThread['t1'].length;
    state = reducer(
      state,
      subagentSpawned({
        threadId: 't1',
        round: 0,
        rowId: 't1:subagent:task-1:researcher',
        taskId: 'task-1',
        agentId: 'researcher',
      })
    );
    expect(state.toolTimelineByThread['t1']).toHaveLength(before);
  });
});

describe('toolArgsDeltaReceived (Phase 3 reducer-side merge)', () => {
  it('creates a running row when args arrive before the tool_call, then appends', () => {
    let state = reducer(
      undefined,
      toolArgsDeltaReceived({
        threadId: 't1',
        round: 0,
        delta: '{"q":',
        toolName: 'search',
        toolCallId: 'c1',
      })
    );
    state = reducer(
      state,
      toolArgsDeltaReceived({ threadId: 't1', round: 0, delta: '"hi"}', toolCallId: 'c1' })
    );
    const rows = state.toolTimelineByThread['t1'];
    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({
      id: 'c1',
      name: 'search',
      status: 'running',
      argsBuffer: '{"q":"hi"}',
    });
  });

  it('falls back to the newest running row of the same name+round when no id matches', () => {
    let state = reducer(
      undefined,
      setToolTimelineForThread({
        threadId: 't1',
        entries: [
          { id: 'r1', name: 'search', round: 0, seq: 0, status: 'running', argsBuffer: '{' },
        ],
      })
    );
    state = reducer(
      state,
      toolArgsDeltaReceived({ threadId: 't1', round: 0, delta: '}', toolName: 'search' })
    );
    expect(state.toolTimelineByThread['t1'][0].argsBuffer).toBe('{}');
  });
});
