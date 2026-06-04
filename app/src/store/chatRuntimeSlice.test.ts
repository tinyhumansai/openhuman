import { configureStore } from '@reduxjs/toolkit';
import { describe, expect, it } from 'vitest';

import chatRuntimeReducer, {
  clearAllChatRuntime,
  clearQueueStatusForThread,
  clearRuntimeForThread,
  type QueueStatus,
  setQueueStatusForThread,
} from './chatRuntimeSlice';

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
