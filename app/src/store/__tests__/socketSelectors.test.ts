import { beforeEach, describe, expect, it } from 'vitest';

import { type CoreState, setCoreStateSnapshot } from '../../lib/coreState/store';
import type { RootState } from '../index';
import { selectSocketId, selectSocketStatus } from '../socketSelectors';

function encodeJwt(payload: Record<string, unknown>): string {
  const header = btoa(JSON.stringify({ alg: 'HS256', typ: 'JWT' }));
  const body = btoa(JSON.stringify(payload));
  return `${header}.${body}.signature`;
}

function makeCoreState(token: string | null): CoreState {
  return {
    isBootstrapping: false,
    isReady: true,
    snapshot: {
      auth: { isAuthenticated: !!token, userId: null, user: null, profileId: null },
      sessionToken: token,
      currentUser: null,
      onboardingCompleted: false,
      analyticsEnabled: false,
      localState: { encryptionKey: null, primaryWalletAddress: null, onboardingTasks: null },
    },
    teams: [],
    teamMembersById: {},
    teamInvitesById: {},
  };
}

function makeState(
  byUser: Record<string, { status: string; socketId: string | null }> = {}
): RootState {
  return { socket: { byUser } } as RootState;
}

describe('selectSocketStatus', () => {
  beforeEach(() => {
    setCoreStateSnapshot(makeCoreState(null));
  });

  it('returns disconnected when no token', () => {
    const state = makeState();
    expect(selectSocketStatus(state)).toBe('disconnected');
  });

  it('returns status from user state based on JWT tgUserId', () => {
    setCoreStateSnapshot(makeCoreState(encodeJwt({ tgUserId: 'tg123' })));
    const state = makeState({ tg123: { status: 'connected', socketId: 'sock-1' } });

    expect(selectSocketStatus(state)).toBe('connected');
  });

  it('returns disconnected when JWT user has no socket state', () => {
    setCoreStateSnapshot(makeCoreState(encodeJwt({ tgUserId: 'tg123' })));
    const state = makeState();

    expect(selectSocketStatus(state)).toBe('disconnected');
  });

  it('uses __pending__ for invalid JWT', () => {
    setCoreStateSnapshot(makeCoreState('not-a-jwt'));
    const state = makeState({ __pending__: { status: 'connecting', socketId: null } });

    expect(selectSocketStatus(state)).toBe('connecting');
  });
});

describe('selectSocketId', () => {
  beforeEach(() => {
    setCoreStateSnapshot(makeCoreState(null));
  });

  it('returns null when no token', () => {
    const state = makeState();
    expect(selectSocketId(state)).toBeNull();
  });

  it('returns socketId from user state', () => {
    setCoreStateSnapshot(makeCoreState(encodeJwt({ tgUserId: 'tg123' })));
    const state = makeState({ tg123: { status: 'connected', socketId: 'sock-abc' } });

    expect(selectSocketId(state)).toBe('sock-abc');
  });
});
