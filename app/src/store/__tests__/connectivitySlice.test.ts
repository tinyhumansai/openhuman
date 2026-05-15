import { describe, expect, it } from 'vitest';

import connectivityReducer, { setBackend, setCore, setInternet } from '../connectivitySlice';

describe('connectivitySlice', () => {
  it('setInternet flips the internet channel and tracks errors only on offline', () => {
    let state = connectivityReducer(undefined, setInternet({ value: 'offline', error: 'no wifi' }));
    expect(state.internet).toBe('offline');
    expect(state.lastError.internet).toBe('no wifi');

    state = connectivityReducer(state, setInternet({ value: 'online' }));
    expect(state.internet).toBe('online');
    expect(state.lastError.internet).toBeUndefined();
  });

  it('setCore flips the core channel and tracks errors only on non-reachable', () => {
    let state = connectivityReducer(
      undefined,
      setCore({ value: 'unreachable', error: 'ECONNREFUSED' })
    );
    expect(state.core).toBe('unreachable');
    expect(state.lastError.core).toBe('ECONNREFUSED');

    state = connectivityReducer(state, setCore({ value: 'reachable' }));
    expect(state.core).toBe('reachable');
    expect(state.lastError.core).toBeUndefined();
  });

  it('setBackend flips the backend channel and tracks errors only on non-connected', () => {
    let state = connectivityReducer(
      undefined,
      setBackend({ value: 'disconnected', error: 'transport close' })
    );
    expect(state.backend).toBe('disconnected');
    expect(state.lastError.backend).toBe('transport close');

    state = connectivityReducer(state, setBackend({ value: 'connected' }));
    expect(state.backend).toBe('connected');
    expect(state.lastError.backend).toBeUndefined();
  });
});
