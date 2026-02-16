import { describe, expect, it } from 'vitest';

import type { RootState } from '../index';
import { selectSocketId, selectSocketStatus } from '../socketSelectors';

// Create a mock JWT with payload { tgUserId: "tg-user-1" }
function encodeJwt(payload: Record<string, unknown>): string {
  const header = btoa(JSON.stringify({ alg: 'HS256', typ: 'JWT' }));
  const body = btoa(JSON.stringify(payload));
  return `${header}.${body}.signature`;
}

function makeState(
  token: string | null,
  byUser: Record<string, { status: string; socketId: string | null }> = {}
): RootState {
  return {
    auth: { token, isOnboardedByUser: {}, isAnalyticsEnabledByUser: {} },
    socket: { byUser },
    user: { user: null, isLoading: false, error: null },
    team: {} as RootState['team'],
    ai: {} as RootState['ai'],
    skills: {} as RootState['skills'],
  } as RootState;
}

describe('selectSocketStatus', () => {
  it('returns disconnected when no token', () => {
    const state = makeState(null);
    expect(selectSocketStatus(state)).toBe('disconnected');
  });

  it('returns status from user state based on JWT tgUserId', () => {
    const token = encodeJwt({ tgUserId: 'tg123' });
    const state = makeState(token, { tg123: { status: 'connected', socketId: 'sock-1' } });
    expect(selectSocketStatus(state)).toBe('connected');
  });

  it('returns disconnected when JWT user has no socket state', () => {
    const token = encodeJwt({ tgUserId: 'tg123' });
    const state = makeState(token, {});
    expect(selectSocketStatus(state)).toBe('disconnected');
  });

  it('uses __pending__ for invalid JWT', () => {
    const state = makeState('not-a-jwt', { __pending__: { status: 'connecting', socketId: null } });
    expect(selectSocketStatus(state)).toBe('connecting');
  });
});

describe('selectSocketId', () => {
  it('returns null when no token', () => {
    const state = makeState(null);
    expect(selectSocketId(state)).toBeNull();
  });

  it('returns socketId from user state', () => {
    const token = encodeJwt({ tgUserId: 'tg123' });
    const state = makeState(token, { tg123: { status: 'connected', socketId: 'sock-abc' } });
    expect(selectSocketId(state)).toBe('sock-abc');
  });
});
