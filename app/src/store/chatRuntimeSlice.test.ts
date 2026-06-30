import { configureStore } from '@reduxjs/toolkit';
import { describe, expect, it } from 'vitest';

import type { AgentRun, AgentRunStatus, PersistedTurnState } from '../types/turnState';
import chatRuntimeReducer, {
  appendProcessingProse,
  clearAllChatRuntime,
  clearQueueStatusForThread,
  clearRuntimeForThread,
  hydrateRuntimeFromRunLedger,
  hydrateRuntimeFromSnapshot,
  hydrateThreadUsage,
  type QueueStatus,
  recordChatTurnUsage,
  resetSessionTokenUsage,
  setQueueStatusForThread,
  setToolTimelineForThread,
} from './chatRuntimeSlice';

function makeRun(id: string, status: AgentRunStatus): AgentRun {
  return {
    id,
    kind: 'subagent',
    status,
    agentId: 'tinyplace_agent',
    metadata: { displayName: 'Tinyplace Agent' },
    startedAt: '2026-06-23T00:00:00Z',
    updatedAt: '2026-06-23T00:00:00Z',
  };
}

function makeInterruptedSnapshot(
  threadId: string,
  toolTimeline: PersistedTurnState['toolTimeline']
): PersistedTurnState {
  return {
    threadId,
    requestId: 'req-1',
    lifecycle: 'interrupted',
    iteration: 3,
    maxIterations: 10,
    streamingText: '',
    thinking: '',
    toolTimeline,
    startedAt: '2026-06-23T00:00:00Z',
    updatedAt: '2026-06-23T00:00:00Z',
  };
}

function makeStore() {
  return configureStore({ reducer: { chatRuntime: chatRuntimeReducer } });
}

describe('chatRuntimeSlice recordChatTurnUsage', () => {
  it('accumulates tokens, cost, and context window across turns', () => {
    const store = makeStore();
    store.dispatch(
      recordChatTurnUsage({
        inputTokens: 1000,
        outputTokens: 200,
        cachedTokens: 50,
        costUsd: 0.012,
        contextWindow: 200_000,
      })
    );
    store.dispatch(
      recordChatTurnUsage({
        inputTokens: 500,
        outputTokens: 100,
        cachedTokens: 10,
        costUsd: 0.008,
        contextWindow: 200_000,
      })
    );
    const usage = store.getState().chatRuntime.sessionTokenUsage;
    expect(usage.inputTokens).toBe(1500);
    expect(usage.outputTokens).toBe(300);
    expect(usage.cachedTokens).toBe(60);
    expect(usage.costUsd).toBeCloseTo(0.02, 6);
    expect(usage.turns).toBe(2);
    expect(usage.contextWindow).toBe(200_000);
    // Context gauge tracks the latest turn's input+output, not the running sum.
    expect(usage.lastTurnContextUsed).toBe(600);
  });

  it('rolls sub-agent spend into a per-archetype breakdown keyed by agentId', () => {
    const store = makeStore();
    store.dispatch(
      recordChatTurnUsage({
        inputTokens: 100,
        outputTokens: 20,
        subAgents: [
          { agentId: 'researcher', inputTokens: 40, outputTokens: 10, costUsd: 0.001 },
          { agentId: 'coder', inputTokens: 80, outputTokens: 30, costUsd: 0.003 },
        ],
      })
    );
    store.dispatch(
      recordChatTurnUsage({
        inputTokens: 50,
        outputTokens: 10,
        subAgents: [{ agentId: 'researcher', inputTokens: 60, outputTokens: 5, costUsd: 0.002 }],
      })
    );
    const subs = store.getState().chatRuntime.sessionTokenUsage.subAgents;
    expect(subs.researcher).toEqual({
      agentId: 'researcher',
      inputTokens: 100,
      outputTokens: 15,
      costUsd: 0.003,
      runs: 2,
    });
    expect(subs.coder.runs).toBe(1);
    expect(subs.coder.inputTokens).toBe(80);
  });

  it('keeps the prior context window when a turn reports an unknown (0) window', () => {
    const store = makeStore();
    store.dispatch(
      recordChatTurnUsage({ inputTokens: 10, outputTokens: 5, contextWindow: 128_000 })
    );
    store.dispatch(recordChatTurnUsage({ inputTokens: 10, outputTokens: 5, contextWindow: 0 }));
    expect(store.getState().chatRuntime.sessionTokenUsage.contextWindow).toBe(128_000);
  });

  it('coerces non-finite / negative inputs to zero', () => {
    const store = makeStore();
    store.dispatch(
      recordChatTurnUsage({ inputTokens: Number.NaN, outputTokens: -50, costUsd: -1 })
    );
    const usage = store.getState().chatRuntime.sessionTokenUsage;
    expect(usage.inputTokens).toBe(0);
    expect(usage.outputTokens).toBe(0);
    expect(usage.costUsd).toBe(0);
    expect(usage.turns).toBe(1);
  });

  it('resetSessionTokenUsage clears all accumulated usage', () => {
    const store = makeStore();
    store.dispatch(
      recordChatTurnUsage({
        inputTokens: 100,
        outputTokens: 20,
        costUsd: 0.01,
        subAgents: [{ agentId: 'researcher', inputTokens: 1, outputTokens: 1, costUsd: 0.001 }],
      })
    );
    store.dispatch(resetSessionTokenUsage());
    const usage = store.getState().chatRuntime.sessionTokenUsage;
    expect(usage.inputTokens).toBe(0);
    expect(usage.costUsd).toBe(0);
    expect(usage.turns).toBe(0);
    expect(usage.subAgents).toEqual({});
  });

  it('routes a turn with a threadId into that thread bucket (and the global)', () => {
    const store = makeStore();
    store.dispatch(
      recordChatTurnUsage({ inputTokens: 100, outputTokens: 20, costUsd: 0.01, threadId: 'thr-a' })
    );
    store.dispatch(
      recordChatTurnUsage({ inputTokens: 50, outputTokens: 10, costUsd: 0.005, threadId: 'thr-b' })
    );
    const { usageByThread, sessionTokenUsage } = store.getState().chatRuntime;
    expect(usageByThread['thr-a'].inputTokens).toBe(100);
    expect(usageByThread['thr-a'].costUsd).toBeCloseTo(0.01, 6);
    expect(usageByThread['thr-b'].inputTokens).toBe(50);
    // Global aggregate still sums both threads.
    expect(sessionTokenUsage.inputTokens).toBe(150);
  });

  it('hydrateThreadUsage seeds a thread bucket and live turns accumulate on top', () => {
    const store = makeStore();
    store.dispatch(
      hydrateThreadUsage({
        threadId: 'thr-a',
        inputTokens: 1000,
        outputTokens: 300,
        cachedTokens: 40,
        costUsd: 0.02,
        turns: 3,
        contextWindow: 1_000_000,
        lastTurnInputTokens: 400,
        lastTurnOutputTokens: 120,
        subAgents: [
          { agentId: 'coder', inputTokens: 300, outputTokens: 80, costUsd: 0.006, runs: 2 },
        ],
      })
    );
    let bucket = store.getState().chatRuntime.usageByThread['thr-a'];
    expect(bucket.inputTokens).toBe(1000);
    expect(bucket.turns).toBe(3);
    expect(bucket.contextWindow).toBe(1_000_000);
    expect(bucket.lastTurnContextUsed).toBe(520);
    // Sub-agent breakdown reconstructed from persisted transcripts.
    expect(bucket.subAgents.coder).toEqual({
      agentId: 'coder',
      inputTokens: 300,
      outputTokens: 80,
      costUsd: 0.006,
      runs: 2,
    });

    // A live turn for the same thread adds on top of the seeded base.
    store.dispatch(
      recordChatTurnUsage({ inputTokens: 200, outputTokens: 50, costUsd: 0.004, threadId: 'thr-a' })
    );
    bucket = store.getState().chatRuntime.usageByThread['thr-a'];
    expect(bucket.inputTokens).toBe(1200);
    expect(bucket.turns).toBe(4);
    expect(bucket.costUsd).toBeCloseTo(0.024, 6);
  });
});

describe('chatRuntimeSlice queue status', () => {
  it('sets queue status for a thread', () => {
    const store = makeStore();
    const status: QueueStatus = { active: true, steers: 1, followups: 2, collects: 0, total: 3 };
    store.dispatch(setQueueStatusForThread({ threadId: 't1', status }));
    expect(store.getState().chatRuntime.queueStatusByThread['t1']).toEqual(status);
  });

  it('clears queue status for a thread', () => {
    const store = makeStore();
    const status: QueueStatus = { active: true, steers: 1, followups: 0, collects: 0, total: 1 };
    store.dispatch(setQueueStatusForThread({ threadId: 't1', status }));
    store.dispatch(clearQueueStatusForThread({ threadId: 't1' }));
    expect(store.getState().chatRuntime.queueStatusByThread['t1']).toBeUndefined();
  });

  it('clearRuntimeForThread removes queue status', () => {
    const store = makeStore();
    const status: QueueStatus = { active: true, steers: 1, followups: 0, collects: 0, total: 1 };
    store.dispatch(setQueueStatusForThread({ threadId: 't1', status }));
    store.dispatch(clearRuntimeForThread({ threadId: 't1' }));
    expect(store.getState().chatRuntime.queueStatusByThread['t1']).toBeUndefined();
  });

  it('clearAllChatRuntime removes all queue statuses', () => {
    const store = makeStore();
    store.dispatch(
      setQueueStatusForThread({
        threadId: 't1',
        status: { active: true, steers: 1, followups: 0, collects: 0, total: 1 },
      })
    );
    store.dispatch(
      setQueueStatusForThread({
        threadId: 't2',
        status: { active: true, steers: 0, followups: 1, collects: 0, total: 1 },
      })
    );
    // Also seed a processing transcript so the clear covers it too (a global
    // reset must not leave stale "View processing" prose behind).
    store.dispatch(
      appendProcessingProse({ threadId: 't1', kind: 'narration', round: 1, delta: 'thinking…' })
    );
    expect(store.getState().chatRuntime.processingByThread.t1).toHaveLength(1);
    store.dispatch(clearAllChatRuntime());
    expect(store.getState().chatRuntime.queueStatusByThread).toEqual({});
    expect(store.getState().chatRuntime.processingByThread).toEqual({});
  });

  it('updates queue status when set again', () => {
    const store = makeStore();
    store.dispatch(
      setQueueStatusForThread({
        threadId: 't1',
        status: { active: true, steers: 1, followups: 0, collects: 0, total: 1 },
      })
    );
    store.dispatch(
      setQueueStatusForThread({
        threadId: 't1',
        status: { active: true, steers: 0, followups: 0, collects: 0, total: 0 },
      })
    );
    expect(store.getState().chatRuntime.queueStatusByThread['t1']).toEqual({
      active: true,
      steers: 0,
      followups: 0,
      collects: 0,
      total: 0,
    });
  });

  it('settles orphaned running rows when hydrating an interrupted snapshot', () => {
    const store = makeStore();
    store.dispatch(
      hydrateRuntimeFromSnapshot({
        snapshot: makeInterruptedSnapshot('t1', [
          {
            id: 't1:subagent:s1:tinyplace_agent',
            name: 'subagent:tinyplace_agent',
            round: 1,
            status: 'running',
            subagent: {
              taskId: 's1',
              agentId: 'tinyplace_agent',
              status: 'running',
              toolCalls: [],
            },
          },
          {
            id: 't1:subagent:s2:tinyplace_agent',
            name: 'subagent:tinyplace_agent',
            round: 1,
            status: 'success',
            subagent: {
              taskId: 's2',
              agentId: 'tinyplace_agent',
              status: 'completed',
              toolCalls: [],
            },
          },
          {
            id: 't1:subagent:s3:tinyplace_agent',
            name: 'subagent:tinyplace_agent',
            round: 1,
            status: 'error',
            subagent: { taskId: 's3', agentId: 'tinyplace_agent', status: 'failed', toolCalls: [] },
          },
        ]),
      })
    );
    const timeline = store.getState().chatRuntime.toolTimelineByThread['t1'];
    // The dangling 'running' row becomes terminal 'cancelled' (no live driver to settle it)…
    expect(timeline[0].status).toBe('cancelled');
    expect(timeline[0].subagent?.status).toBe('cancelled');
    // …while already-terminal rows are left untouched.
    expect(timeline[1].status).toBe('success');
    expect(timeline[1].subagent?.status).toBe('completed');
    expect(timeline[2].status).toBe('error');
    expect(timeline[2].subagent?.status).toBe('failed');
  });

  it('renders interrupted run-ledger rows as muted (cancelled), reserving error for failed', () => {
    const store = makeStore();
    store.dispatch(
      hydrateRuntimeFromRunLedger({
        threadId: 't1',
        runs: [
          makeRun('sub-interrupted', 'interrupted'),
          makeRun('sub-failed', 'failed'),
          makeRun('sub-completed', 'completed'),
        ],
      })
    );
    const byId = Object.fromEntries(
      store.getState().chatRuntime.toolTimelineByThread['t1'].map(e => [e.id, e.status])
    );
    // Orphaned (interrupted) background runs are terminal but NOT user-facing
    // errors — muted, not alarming red.
    expect(byId['subagent:sub-interrupted']).toBe('cancelled');
    // A genuine failure still surfaces as an error.
    expect(byId['subagent:sub-failed']).toBe('error');
    expect(byId['subagent:sub-completed']).toBe('success');
  });

  it('settles the parent row but preserves an awaiting_user subagent on interrupt', () => {
    const store = makeStore();
    store.dispatch(
      hydrateRuntimeFromSnapshot({
        snapshot: makeInterruptedSnapshot('t2', [
          {
            id: 't2:subagent:s1:researcher',
            name: 'subagent:researcher',
            round: 1,
            // Core keeps the row `running` while the child is paused for the user.
            status: 'running',
            subagent: {
              taskId: 's1',
              agentId: 'researcher',
              status: 'awaiting_user',
              workerThreadId: 'worker-1',
              toolCalls: [],
            },
          },
        ]),
      })
    );
    const row = store.getState().chatRuntime.toolTimelineByThread['t2'][0];
    // The row stops pulsing (status drives agentNameTone)…
    expect(row.status).toBe('cancelled');
    // …but the truthful "was awaiting user" child state is kept, not clobbered.
    expect(row.subagent?.status).toBe('awaiting_user');
    expect(row.subagent?.workerThreadId).toBe('worker-1');
  });

  it('isolates queue status across threads', () => {
    const store = makeStore();
    store.dispatch(
      setQueueStatusForThread({
        threadId: 't1',
        status: { active: true, steers: 1, followups: 0, collects: 0, total: 1 },
      })
    );
    store.dispatch(
      setQueueStatusForThread({
        threadId: 't2',
        status: { active: true, steers: 0, followups: 2, collects: 0, total: 2 },
      })
    );
    expect(store.getState().chatRuntime.queueStatusByThread['t1']?.steers).toBe(1);
    expect(store.getState().chatRuntime.queueStatusByThread['t2']?.followups).toBe(2);
  });
});

describe('hydrateRuntimeFromSnapshot — sub-agent prose persistence', () => {
  it('carries live sub-agent thoughts across rehydration (matched by taskId)', () => {
    const store = makeStore();
    // Live in-memory row: sub-agent with streamed reasoning + a tool call.
    // Live and persisted rows use different entry ids, so the merge matches
    // on the sub-agent taskId.
    store.dispatch(
      setToolTimelineForThread({
        threadId: 't9',
        entries: [
          {
            id: 't9:subagent:task-x:spawn_subagent',
            name: 'subagent:researcher',
            round: 1,
            status: 'running',
            subagent: {
              taskId: 'task-x',
              agentId: 'researcher',
              toolCalls: [],
              transcript: [
                { kind: 'thinking', iteration: 1, text: 'let me search the inbox' },
                {
                  kind: 'tool',
                  iteration: 1,
                  callId: 'c1',
                  toolName: 'web_search',
                  status: 'success',
                },
              ],
            },
          },
        ],
      })
    );

    // Snapshot rebuilds the sub-agent transcript from tool calls only (no
    // prose) and uses the persisted entry id `subagent:<taskId>`.
    const snapshot: PersistedTurnState = {
      threadId: 't9',
      requestId: 'req-1',
      lifecycle: 'streaming',
      iteration: 1,
      maxIterations: 10,
      streamingText: '',
      thinking: '',
      toolTimeline: [
        {
          id: 'subagent:task-x',
          name: 'subagent:researcher',
          round: 1,
          status: 'running',
          subagent: {
            taskId: 'task-x',
            agentId: 'researcher',
            toolCalls: [{ callId: 'c1', toolName: 'web_search', status: 'success' }],
          },
        },
      ],
      startedAt: '2026-06-23T00:00:00Z',
      updatedAt: '2026-06-23T00:00:00Z',
    };

    store.dispatch(hydrateRuntimeFromSnapshot({ snapshot }));

    const row = store
      .getState()
      .chatRuntime.toolTimelineByThread['t9'].find(e => e.subagent?.taskId === 'task-x');
    const transcript = row?.subagent?.transcript ?? [];
    // The streamed thought survives the rehydration instead of being clobbered
    // by the prose-less snapshot.
    const thinking = transcript.find(i => i.kind === 'thinking');
    expect(thinking && 'text' in thinking ? thinking.text : undefined).toBe(
      'let me search the inbox'
    );
  });

  it('replays a persisted sub-agent transcript on a settled turn (no live data)', () => {
    const store = makeStore();
    // No live entries seeded — this is the settled / reloaded case. The
    // snapshot itself now carries the sub-agent prose transcript.
    const snapshot: PersistedTurnState = {
      threadId: 't10',
      requestId: 'req-1',
      lifecycle: 'completed',
      iteration: 2,
      maxIterations: 10,
      streamingText: '',
      thinking: '',
      toolTimeline: [
        {
          id: 'subagent:task-y',
          name: 'subagent:researcher',
          round: 1,
          status: 'success',
          subagent: {
            taskId: 'task-y',
            agentId: 'researcher',
            toolCalls: [{ callId: 'c1', toolName: 'web_search', status: 'success' }],
            transcript: [
              { kind: 'thinking', iteration: 1, text: 'planning the search' },
              {
                kind: 'tool',
                iteration: 1,
                callId: 'c1',
                toolName: 'web_search',
                status: 'success',
              },
              { kind: 'text', iteration: 1, text: 'here is the summary' },
            ],
          },
        },
      ],
      startedAt: '2026-06-23T00:00:00Z',
      updatedAt: '2026-06-23T00:00:00Z',
    };

    store.dispatch(hydrateRuntimeFromSnapshot({ snapshot }));

    const row = store
      .getState()
      .chatRuntime.toolTimelineByThread['t10'].find(e => e.subagent?.taskId === 'task-y');
    const transcript = row?.subagent?.transcript ?? [];
    // The persisted prose survives a reload with no in-memory live data.
    expect(transcript.map(i => i.kind)).toEqual(['thinking', 'tool', 'text']);
    const thinking = transcript[0];
    expect(thinking.kind === 'thinking' ? thinking.text : undefined).toBe('planning the search');
    const text = transcript[2];
    expect(text.kind === 'text' ? text.text : undefined).toBe('here is the summary');
  });
});
