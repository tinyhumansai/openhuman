import { describe, expect, it } from 'vitest';

import companionReducer, {
  clearSession,
  type CompanionSliceState,
  selectCompanionConfig,
  selectCompanionLastError,
  selectCompanionSessionActive,
  selectCompanionState,
  setCompanionState,
  setConfig,
  setLastError,
  setSessionActive,
} from './companionSlice';
import { resetUserScopedState } from './resetActions';

const initialState: CompanionSliceState = {
  sessionActive: false,
  state: 'idle',
  sessionId: null,
  config: null,
  lastError: null,
};

describe('companionSlice', () => {
  it('starts with idle initialState', () => {
    const state = companionReducer(undefined, { type: 'unknown' });
    expect(state).toEqual(initialState);
  });

  it('setCompanionState updates state, sessionId, and sessionActive', () => {
    const state = companionReducer(
      initialState,
      setCompanionState({ session_id: 'sess-1', state: 'listening', previous_state: 'idle' })
    );
    expect(state.state).toBe('listening');
    expect(state.sessionId).toBe('sess-1');
    expect(state.sessionActive).toBe(true);
  });

  it('setCompanionState to idle clears sessionActive', () => {
    const active: CompanionSliceState = {
      ...initialState,
      sessionActive: true,
      state: 'speaking',
      sessionId: 'sess-1',
    };
    const state = companionReducer(
      active,
      setCompanionState({ session_id: 'sess-1', state: 'idle', previous_state: 'speaking' })
    );
    expect(state.sessionActive).toBe(false);
    expect(state.state).toBe('idle');
  });

  it('setCompanionState to error stores message', () => {
    const state = companionReducer(
      initialState,
      setCompanionState({
        session_id: 'sess-1',
        state: 'error',
        previous_state: 'thinking',
        message: 'LLM timeout',
      })
    );
    expect(state.state).toBe('error');
    expect(state.lastError).toBe('LLM timeout');
    expect(state.sessionActive).toBe(false);
  });

  it('setCompanionState clears stale lastError on recovery to non-error state', () => {
    const withError: CompanionSliceState = { ...initialState, lastError: 'old failure' };
    const state = companionReducer(
      withError,
      setCompanionState({ session_id: 'sess-1', state: 'listening', previous_state: 'error' })
    );
    expect(state.lastError).toBeNull();
    expect(state.state).toBe('listening');
  });

  it('setSessionActive sets active flag and sessionId', () => {
    const state = companionReducer(
      initialState,
      setSessionActive({ active: true, sessionId: 'sess-2' })
    );
    expect(state.sessionActive).toBe(true);
    expect(state.sessionId).toBe('sess-2');
  });

  it('setSessionActive(false) resets state to idle', () => {
    const active: CompanionSliceState = {
      ...initialState,
      sessionActive: true,
      state: 'speaking',
      sessionId: 'sess-1',
    };
    const state = companionReducer(active, setSessionActive({ active: false, sessionId: null }));
    expect(state.sessionActive).toBe(false);
    expect(state.state).toBe('idle');
  });

  it('setConfig stores config object', () => {
    const config = {
      hotkey: 'ctrl+space',
      activation_mode: 'push',
      ttl_secs: 3600,
      capture_screen: true,
      include_app_context: true,
    };
    const state = companionReducer(initialState, setConfig(config));
    expect(state.config).toEqual(config);
  });

  it('setLastError stores error string', () => {
    const state = companionReducer(initialState, setLastError('oops'));
    expect(state.lastError).toBe('oops');
  });

  it('setLastError(null) clears error', () => {
    const withError: CompanionSliceState = { ...initialState, lastError: 'old error' };
    const state = companionReducer(withError, setLastError(null));
    expect(state.lastError).toBeNull();
  });

  it('clearSession returns to initialState', () => {
    const active: CompanionSliceState = {
      sessionActive: true,
      state: 'speaking',
      sessionId: 'sess-1',
      config: {
        hotkey: 'ctrl+space',
        activation_mode: 'push',
        ttl_secs: 3600,
        capture_screen: true,
        include_app_context: true,
      },
      lastError: 'some error',
    };
    const state = companionReducer(active, clearSession());
    expect(state).toEqual(initialState);
  });

  it('resetUserScopedState resets to initialState', () => {
    const active: CompanionSliceState = {
      ...initialState,
      sessionActive: true,
      sessionId: 'sess-1',
    };
    const state = companionReducer(active, resetUserScopedState());
    expect(state).toEqual(initialState);
  });
});

describe('companion selectors', () => {
  const root = {
    companion: {
      sessionActive: true,
      state: 'speaking' as const,
      sessionId: 'sess-1',
      config: {
        hotkey: 'ctrl+space',
        activation_mode: 'push',
        ttl_secs: 3600,
        capture_screen: true,
        include_app_context: true,
      },
      lastError: 'err',
    },
  };

  it('selectCompanionState returns state', () => {
    expect(selectCompanionState(root)).toBe('speaking');
  });

  it('selectCompanionSessionActive returns sessionActive', () => {
    expect(selectCompanionSessionActive(root)).toBe(true);
  });

  it('selectCompanionConfig returns config', () => {
    expect(selectCompanionConfig(root)).toEqual(root.companion.config);
  });

  it('selectCompanionLastError returns lastError', () => {
    expect(selectCompanionLastError(root)).toBe('err');
  });
});
