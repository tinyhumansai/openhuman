import { describe, expect, it } from 'vitest';

import { parseCompanionStateChangedEvent } from '../socketService';

describe('parseCompanionStateChangedEvent', () => {
  it('returns null for non-object inputs', () => {
    expect(parseCompanionStateChangedEvent(null)).toBeNull();
    expect(parseCompanionStateChangedEvent(undefined)).toBeNull();
    expect(parseCompanionStateChangedEvent(42)).toBeNull();
    expect(parseCompanionStateChangedEvent('listening')).toBeNull();
  });

  it('returns null when session_id is missing or non-string', () => {
    expect(parseCompanionStateChangedEvent({ state: 'listening' })).toBeNull();
    expect(parseCompanionStateChangedEvent({ session_id: 42, state: 'listening' })).toBeNull();
  });

  it('returns null when state is missing or not in the enum', () => {
    expect(parseCompanionStateChangedEvent({ session_id: 's1' })).toBeNull();
    expect(parseCompanionStateChangedEvent({ session_id: 's1', state: 'unknown' })).toBeNull();
    expect(parseCompanionStateChangedEvent({ session_id: 's1', state: 7 })).toBeNull();
  });

  it('accepts a valid payload and round-trips all fields', () => {
    const event = parseCompanionStateChangedEvent({
      session_id: 'sess-1',
      state: 'speaking',
      previous_state: 'thinking',
      message: 'all good',
    });
    expect(event).toEqual({
      session_id: 'sess-1',
      state: 'speaking',
      previous_state: 'thinking',
      message: 'all good',
    });
  });

  it("defaults previous_state to 'idle' when missing or invalid", () => {
    const missing = parseCompanionStateChangedEvent({ session_id: 's', state: 'listening' });
    expect(missing?.previous_state).toBe('idle');

    const invalid = parseCompanionStateChangedEvent({
      session_id: 's',
      state: 'listening',
      previous_state: 'banana',
    });
    expect(invalid?.previous_state).toBe('idle');
  });

  it('strips non-string message values', () => {
    const event = parseCompanionStateChangedEvent({ session_id: 's', state: 'error', message: 42 });
    expect(event?.message).toBeUndefined();
  });

  it('accepts every valid state value', () => {
    for (const state of ['idle', 'listening', 'thinking', 'speaking', 'pointing', 'error']) {
      const event = parseCompanionStateChangedEvent({ session_id: 's', state });
      expect(event?.state).toBe(state);
    }
  });
});
