import { describe, expect, it } from 'vitest';

import reducer, {
  beginInferenceTurn,
  clearAllChatRuntime,
  clearQueueStatusForThread,
  clearRuntimeForThread,
  endInferenceTurn,
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
