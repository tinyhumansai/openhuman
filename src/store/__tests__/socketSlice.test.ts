import { configureStore } from '@reduxjs/toolkit';
import { describe, expect, it } from 'vitest';

import socketReducer, { resetForUser, setSocketIdForUser, setStatusForUser } from '../socketSlice';

function createStore() {
  return configureStore({ reducer: { socket: socketReducer } });
}

describe('socketSlice', () => {
  it('starts with empty byUser map', () => {
    const store = createStore();
    expect(store.getState().socket.byUser).toEqual({});
  });

  it('sets status for a user', () => {
    const store = createStore();
    store.dispatch(setStatusForUser({ userId: 'u1', status: 'connecting' }));
    expect(store.getState().socket.byUser.u1.status).toBe('connecting');
    expect(store.getState().socket.byUser.u1.socketId).toBeNull();
  });

  it('clears socketId when status is disconnected', () => {
    const store = createStore();
    store.dispatch(setStatusForUser({ userId: 'u1', status: 'connected' }));
    store.dispatch(setSocketIdForUser({ userId: 'u1', socketId: 'sock-123' }));
    expect(store.getState().socket.byUser.u1.socketId).toBe('sock-123');

    store.dispatch(setStatusForUser({ userId: 'u1', status: 'disconnected' }));
    expect(store.getState().socket.byUser.u1.socketId).toBeNull();
  });

  it('sets socketId for a user', () => {
    const store = createStore();
    store.dispatch(setSocketIdForUser({ userId: 'u1', socketId: 'sock-abc' }));
    expect(store.getState().socket.byUser.u1.socketId).toBe('sock-abc');
  });

  it('resets user state', () => {
    const store = createStore();
    store.dispatch(setStatusForUser({ userId: 'u1', status: 'connected' }));
    store.dispatch(setSocketIdForUser({ userId: 'u1', socketId: 'sock-123' }));
    store.dispatch(resetForUser({ userId: 'u1' }));

    expect(store.getState().socket.byUser.u1.status).toBe('disconnected');
    expect(store.getState().socket.byUser.u1.socketId).toBeNull();
  });

  it('handles multiple users independently', () => {
    const store = createStore();
    store.dispatch(setStatusForUser({ userId: 'u1', status: 'connected' }));
    store.dispatch(setStatusForUser({ userId: 'u2', status: 'connecting' }));

    expect(store.getState().socket.byUser.u1.status).toBe('connected');
    expect(store.getState().socket.byUser.u2.status).toBe('connecting');
  });
});
