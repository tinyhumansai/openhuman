import { describe, expect, it } from 'vitest';

import type { PersistedTurnState } from '../../types/turnState';
import reducer, {
  beginInferenceTurn,
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
        entries: [{ id: 'call-1', name: 'search', round: 1, status: 'running' }],
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
