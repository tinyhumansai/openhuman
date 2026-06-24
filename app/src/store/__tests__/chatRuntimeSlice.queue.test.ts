import { describe, expect, it } from 'vitest';

import reducer, {
  beginInferenceTurn,
  clearAllChatRuntime,
  clearFollowupsForThread,
  clearQueueStatusForThread,
  clearRuntimeForThread,
  endInferenceTurn,
  enqueueFollowup,
  removeFollowup,
  setQueueStatusForThread,
} from '../chatRuntimeSlice';

describe('chatRuntimeSlice — queue status', () => {
  it('stores and clears per-thread queue status', () => {
    const withStatus = reducer(
      undefined,
      setQueueStatusForThread({
        threadId: 'thread-1',
        status: { active: true, steers: 1, followups: 0, collects: 2, total: 3 },
      })
    );

    expect(withStatus.queueStatusByThread['thread-1']).toEqual({
      active: true,
      steers: 1,
      followups: 0,
      collects: 2,
      total: 3,
    });

    const cleared = reducer(withStatus, clearQueueStatusForThread({ threadId: 'thread-1' }));
    expect(cleared.queueStatusByThread['thread-1']).toBeUndefined();
  });

  it('updates queue status in place', () => {
    let state = reducer(
      undefined,
      setQueueStatusForThread({
        threadId: 'thread-1',
        status: { active: true, steers: 1, followups: 0, collects: 0, total: 1 },
      })
    );
    state = reducer(
      state,
      setQueueStatusForThread({
        threadId: 'thread-1',
        status: { active: true, steers: 2, followups: 1, collects: 0, total: 3 },
      })
    );

    expect(state.queueStatusByThread['thread-1']?.total).toBe(3);
    expect(state.queueStatusByThread['thread-1']?.steers).toBe(2);
  });

  it('clearRuntimeForThread removes queue status', () => {
    let state = reducer(
      undefined,
      setQueueStatusForThread({
        threadId: 'thread-1',
        status: { active: true, steers: 1, followups: 0, collects: 0, total: 1 },
      })
    );
    state = reducer(state, beginInferenceTurn({ threadId: 'thread-1' }));
    state = reducer(state, clearRuntimeForThread({ threadId: 'thread-1' }));

    expect(state.queueStatusByThread['thread-1']).toBeUndefined();
    expect(state.inferenceTurnLifecycleByThread['thread-1']).toBeUndefined();
  });

  it('clearAllChatRuntime removes all queue statuses', () => {
    let state = reducer(
      undefined,
      setQueueStatusForThread({
        threadId: 'thread-1',
        status: { active: true, steers: 1, followups: 0, collects: 0, total: 1 },
      })
    );
    state = reducer(
      state,
      setQueueStatusForThread({
        threadId: 'thread-2',
        status: { active: true, steers: 0, followups: 1, collects: 0, total: 1 },
      })
    );
    state = reducer(state, clearAllChatRuntime());

    expect(Object.keys(state.queueStatusByThread)).toHaveLength(0);
  });

  it('inactive queue status has zero counts', () => {
    const state = reducer(
      undefined,
      setQueueStatusForThread({
        threadId: 'thread-1',
        status: { active: false, steers: 0, followups: 0, collects: 0, total: 0 },
      })
    );

    expect(state.queueStatusByThread['thread-1']?.active).toBe(false);
    expect(state.queueStatusByThread['thread-1']?.total).toBe(0);
  });

  it('endInferenceTurn does not clear queue status', () => {
    let state = reducer(
      undefined,
      setQueueStatusForThread({
        threadId: 'thread-1',
        status: { active: true, steers: 1, followups: 0, collects: 0, total: 1 },
      })
    );
    state = reducer(state, beginInferenceTurn({ threadId: 'thread-1' }));
    state = reducer(state, endInferenceTurn({ threadId: 'thread-1' }));

    expect(state.queueStatusByThread['thread-1']).toBeDefined();
  });
});

describe('chatRuntimeSlice — queued follow-ups', () => {
  const enq = (threadId: string, id: string, text: string) =>
    enqueueFollowup({
      threadId,
      message: {
        id,
        content: text,
        type: 'text',
        extraMetadata: {},
        sender: 'user',
        createdAt: '2026-01-01T00:00:00.000Z',
      },
      label: text,
    });

  it('enqueues follow-ups in order per thread', () => {
    let state = reducer(undefined, enq('t1', 'a', 'first'));
    state = reducer(state, enq('t1', 'b', 'second'));

    expect(state.queuedFollowupsByThread['t1'].map(f => f.message.id)).toEqual(['a', 'b']);
    expect(state.queuedFollowupsByThread['t1'].map(f => f.label)).toEqual(['first', 'second']);
    expect(state.queuedFollowupsByThread['t1'][0].message.content).toBe('first');
  });

  it('keeps follow-up queues isolated per thread', () => {
    let state = reducer(undefined, enq('t1', 'a', 'one'));
    state = reducer(state, enq('t2', 'b', 'two'));

    expect(state.queuedFollowupsByThread['t1']).toHaveLength(1);
    expect(state.queuedFollowupsByThread['t2']).toHaveLength(1);
  });

  it('removeFollowup drops one entry by message id and prunes empty buckets', () => {
    let state = reducer(undefined, enq('t1', 'a', 'one'));
    state = reducer(state, enq('t1', 'b', 'two'));

    state = reducer(state, removeFollowup({ threadId: 't1', id: 'a' }));
    expect(state.queuedFollowupsByThread['t1'].map(f => f.message.id)).toEqual(['b']);

    state = reducer(state, removeFollowup({ threadId: 't1', id: 'b' }));
    expect(state.queuedFollowupsByThread['t1']).toBeUndefined();
  });

  it('clearFollowupsForThread drops all entries for the thread', () => {
    let state = reducer(undefined, enq('t1', 'a', 'one'));
    state = reducer(state, clearFollowupsForThread({ threadId: 't1' }));

    expect(state.queuedFollowupsByThread['t1']).toBeUndefined();
  });

  it('endInferenceTurn clears the thread follow-up queue (it is being dispatched)', () => {
    let state = reducer(undefined, enq('t1', 'a', 'one'));
    state = reducer(state, beginInferenceTurn({ threadId: 't1' }));
    state = reducer(state, endInferenceTurn({ threadId: 't1' }));

    expect(state.queuedFollowupsByThread['t1']).toBeUndefined();
  });

  it('clearRuntimeForThread and clearAllChatRuntime drop follow-up queues', () => {
    let state = reducer(undefined, enq('t1', 'a', 'one'));
    state = reducer(state, enq('t2', 'b', 'two'));

    const perThread = reducer(state, clearRuntimeForThread({ threadId: 't1' }));
    expect(perThread.queuedFollowupsByThread['t1']).toBeUndefined();
    expect(perThread.queuedFollowupsByThread['t2']).toBeDefined();

    const all = reducer(state, clearAllChatRuntime());
    expect(Object.keys(all.queuedFollowupsByThread)).toHaveLength(0);
  });
});
