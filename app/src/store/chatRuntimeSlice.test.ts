import { configureStore } from '@reduxjs/toolkit';
import { describe, expect, it } from 'vitest';

import type { AgentRun, AgentRunStatus, PersistedTurnState } from '../types/turnState';
import chatRuntimeReducer, {
  clearAllChatRuntime,
  clearQueueStatusForThread,
  clearRuntimeForThread,
  hydrateRuntimeFromRunLedger,
  hydrateRuntimeFromSnapshot,
  type QueueStatus,
  setQueueStatusForThread,
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
    store.dispatch(clearAllChatRuntime());
    expect(store.getState().chatRuntime.queueStatusByThread).toEqual({});
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
