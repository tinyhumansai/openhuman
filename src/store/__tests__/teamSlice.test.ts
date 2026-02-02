import { describe, expect, it, vi } from 'vitest';

import type { TeamInvite, TeamMember, TeamWithRole } from '../../types/team';
import reducer, { clearTeamState, fetchInvites, fetchMembers, fetchTeams } from '../teamSlice';

// Mock the teamApi module
vi.mock('../../services/api/teamApi', () => ({
  teamApi: { getTeams: vi.fn(), getMembers: vi.fn(), getInvites: vi.fn() },
}));

const mockTeam: TeamWithRole = {
  team: {
    _id: 'team-1',
    name: 'Test Team',
    slug: 'test-team',
    createdBy: 'user-1',
    isPersonal: false,
    maxMembers: 10,
    subscription: { plan: 'FREE', hasActiveSubscription: false },
    usage: { dailyTokenLimit: 1000, remainingTokens: 500, activeSessionCount: 1 },
    createdAt: '2026-01-01T00:00:00Z',
    updatedAt: '2026-01-01T00:00:00Z',
  },
  role: 'ADMIN',
};

const mockMember: TeamMember = {
  _id: 'member-1',
  user: { _id: 'user-1', firstName: 'John', lastName: 'Doe', username: 'johndoe' },
  role: 'ADMIN',
  joinedAt: '2026-01-01T00:00:00Z',
};

const mockInvite: TeamInvite = {
  _id: 'invite-1',
  code: 'ABC123',
  createdBy: 'user-1',
  expiresAt: '2026-12-31T00:00:00Z',
  maxUses: 10,
  currentUses: 2,
  usageHistory: [],
};

describe('teamSlice', () => {
  const initialState = reducer(undefined, { type: '@@INIT' });

  describe('initial state', () => {
    it('should have empty teams array', () => {
      expect(initialState.teams).toEqual([]);
    });

    it('should have empty members array', () => {
      expect(initialState.members).toEqual([]);
    });

    it('should have empty invites array', () => {
      expect(initialState.invites).toEqual([]);
    });

    it('should not be loading', () => {
      expect(initialState.isLoading).toBe(false);
    });

    it('should have null error', () => {
      expect(initialState.error).toBeNull();
    });
  });

  describe('clearTeamState', () => {
    it('should reset to initial state', () => {
      const populated = {
        teams: [mockTeam],
        members: [mockMember],
        invites: [mockInvite],
        isLoading: true,
        error: 'some error',
      };
      const cleared = reducer(populated, clearTeamState());
      expect(cleared).toEqual(initialState);
    });
  });

  describe('fetchTeams', () => {
    it('should set isLoading on pending', () => {
      const state = reducer(initialState, fetchTeams.pending('', undefined));
      expect(state.isLoading).toBe(true);
      expect(state.error).toBeNull();
    });

    it('should populate teams on fulfilled', () => {
      const teams = [mockTeam];
      const state = reducer(initialState, fetchTeams.fulfilled(teams, '', undefined));
      expect(state.isLoading).toBe(false);
      expect(state.teams).toEqual(teams);
    });

    it('should set error on rejected', () => {
      const state = reducer(
        initialState,
        fetchTeams.rejected(null, '', undefined, 'Network error')
      );
      expect(state.isLoading).toBe(false);
      expect(state.error).toBe('Network error');
    });

    it('should clear previous error on pending', () => {
      const withError = { ...initialState, error: 'old error' };
      const state = reducer(withError, fetchTeams.pending('', undefined));
      expect(state.error).toBeNull();
    });
  });

  describe('fetchMembers', () => {
    it('should populate members on fulfilled', () => {
      const members = [mockMember];
      const state = reducer(initialState, fetchMembers.fulfilled(members, '', 'team-1'));
      expect(state.members).toEqual(members);
    });

    it('should set error on rejected', () => {
      const state = reducer(initialState, fetchMembers.rejected(null, '', 'team-1', 'Forbidden'));
      expect(state.error).toBe('Forbidden');
    });

    it('should clear error on pending', () => {
      const withError = { ...initialState, error: 'old' };
      const state = reducer(withError, fetchMembers.pending('', 'team-1'));
      expect(state.error).toBeNull();
    });
  });

  describe('fetchInvites', () => {
    it('should populate invites on fulfilled', () => {
      const invites = [mockInvite];
      const state = reducer(initialState, fetchInvites.fulfilled(invites, '', 'team-1'));
      expect(state.invites).toEqual(invites);
    });

    it('should set error on rejected', () => {
      const state = reducer(initialState, fetchInvites.rejected(null, '', 'team-1', 'Not found'));
      expect(state.error).toBe('Not found');
    });

    it('should clear error on pending', () => {
      const withError = { ...initialState, error: 'old' };
      const state = reducer(withError, fetchInvites.pending('', 'team-1'));
      expect(state.error).toBeNull();
    });
  });
});
