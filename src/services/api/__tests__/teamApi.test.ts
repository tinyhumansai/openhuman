import { beforeEach, describe, expect, it, vi } from 'vitest';

import { teamApi } from '../teamApi';

// Mock the apiClient module
const mockGet = vi.fn();
const mockPost = vi.fn();
const mockPut = vi.fn();
const mockDelete = vi.fn();

vi.mock('../../apiClient', () => ({
  apiClient: {
    get: (...args: unknown[]) => mockGet(...args),
    post: (...args: unknown[]) => mockPost(...args),
    put: (...args: unknown[]) => mockPut(...args),
    delete: (...args: unknown[]) => mockDelete(...args),
  },
}));

describe('teamApi', () => {
  beforeEach(() => {
    mockGet.mockReset();
    mockPost.mockReset();
    mockPut.mockReset();
    mockDelete.mockReset();
  });

  describe('getTeams', () => {
    it('should call GET /teams and return data', async () => {
      const teams = [{ team: { _id: 't1', name: 'Team 1' }, role: 'ADMIN' }];
      mockGet.mockResolvedValue({ success: true, data: teams });

      const result = await teamApi.getTeams();

      expect(mockGet).toHaveBeenCalledWith('/teams');
      expect(result).toEqual(teams);
    });

    it('should propagate errors', async () => {
      mockGet.mockRejectedValue({ success: false, error: 'Unauthorized' });

      await expect(teamApi.getTeams()).rejects.toEqual({ success: false, error: 'Unauthorized' });
    });
  });

  describe('getTeam', () => {
    it('should call GET /teams/:teamId', async () => {
      const team = { _id: 't1', name: 'Team 1' };
      mockGet.mockResolvedValue({ success: true, data: team });

      const result = await teamApi.getTeam('t1');

      expect(mockGet).toHaveBeenCalledWith('/teams/t1');
      expect(result).toEqual(team);
    });
  });

  describe('createTeam', () => {
    it('should call POST /teams with name', async () => {
      const team = { _id: 't2', name: 'New Team' };
      mockPost.mockResolvedValue({ success: true, data: team });

      const result = await teamApi.createTeam('New Team');

      expect(mockPost).toHaveBeenCalledWith('/teams', { name: 'New Team' });
      expect(result).toEqual(team);
    });
  });

  describe('updateTeam', () => {
    it('should call PUT /teams/:teamId with data', async () => {
      const team = { _id: 't1', name: 'Updated' };
      mockPut.mockResolvedValue({ success: true, data: team });

      const result = await teamApi.updateTeam('t1', { name: 'Updated' });

      expect(mockPut).toHaveBeenCalledWith('/teams/t1', { name: 'Updated' });
      expect(result).toEqual(team);
    });
  });

  describe('deleteTeam', () => {
    it('should call DELETE /teams/:teamId', async () => {
      mockDelete.mockResolvedValue({ success: true, data: null });

      await teamApi.deleteTeam('t1');

      expect(mockDelete).toHaveBeenCalledWith('/teams/t1');
    });
  });

  describe('switchTeam', () => {
    it('should call POST /teams/:teamId/switch', async () => {
      mockPost.mockResolvedValue({ success: true, data: null });

      await teamApi.switchTeam('t1');

      expect(mockPost).toHaveBeenCalledWith('/teams/t1/switch');
    });
  });

  describe('getMembers', () => {
    it('should call GET /teams/:teamId/members', async () => {
      const members = [
        {
          _id: 'm1',
          user: { _id: 'u1', firstName: 'John' },
          role: 'ADMIN',
          joinedAt: '2026-01-01',
        },
      ];
      mockGet.mockResolvedValue({ success: true, data: members });

      const result = await teamApi.getMembers('t1');

      expect(mockGet).toHaveBeenCalledWith('/teams/t1/members');
      expect(result).toEqual(members);
    });
  });

  describe('removeMember', () => {
    it('should call DELETE /teams/:teamId/members/:userId', async () => {
      mockDelete.mockResolvedValue({ success: true, data: null });

      await teamApi.removeMember('t1', 'u2');

      expect(mockDelete).toHaveBeenCalledWith('/teams/t1/members/u2');
    });
  });

  describe('changeMemberRole', () => {
    it('should call PUT /teams/:teamId/members/:userId/role', async () => {
      mockPut.mockResolvedValue({ success: true, data: null });

      await teamApi.changeMemberRole('t1', 'u2', 'BILLING_MANAGER');

      expect(mockPut).toHaveBeenCalledWith('/teams/t1/members/u2/role', {
        role: 'BILLING_MANAGER',
      });
    });
  });

  describe('leaveTeam', () => {
    it('should call POST /teams/:teamId/leave', async () => {
      mockPost.mockResolvedValue({ success: true, data: null });

      await teamApi.leaveTeam('t1');

      expect(mockPost).toHaveBeenCalledWith('/teams/t1/leave');
    });
  });

  describe('createInvite', () => {
    it('should call POST /teams/:teamId/invites without opts', async () => {
      const invite = { _id: 'inv1', code: 'ABC123' };
      mockPost.mockResolvedValue({ success: true, data: invite });

      const result = await teamApi.createInvite('t1');

      expect(mockPost).toHaveBeenCalledWith('/teams/t1/invites', undefined);
      expect(result).toEqual(invite);
    });

    it('should pass options when provided', async () => {
      const invite = { _id: 'inv2', code: 'XYZ789' };
      mockPost.mockResolvedValue({ success: true, data: invite });

      await teamApi.createInvite('t1', { maxUses: 5, expiresInDays: 7 });

      expect(mockPost).toHaveBeenCalledWith('/teams/t1/invites', { maxUses: 5, expiresInDays: 7 });
    });
  });

  describe('getInvites', () => {
    it('should call GET /teams/:teamId/invites', async () => {
      const invites = [{ _id: 'inv1', code: 'ABC123' }];
      mockGet.mockResolvedValue({ success: true, data: invites });

      const result = await teamApi.getInvites('t1');

      expect(mockGet).toHaveBeenCalledWith('/teams/t1/invites');
      expect(result).toEqual(invites);
    });
  });

  describe('revokeInvite', () => {
    it('should call DELETE /teams/:teamId/invites/:inviteId', async () => {
      mockDelete.mockResolvedValue({ success: true, data: null });

      await teamApi.revokeInvite('t1', 'inv1');

      expect(mockDelete).toHaveBeenCalledWith('/teams/t1/invites/inv1');
    });
  });

  describe('joinTeam', () => {
    it('should call POST /teams/join with code in body', async () => {
      const team = { _id: 't3', name: 'Joined Team' };
      mockPost.mockResolvedValue({ success: true, data: team });

      const result = await teamApi.joinTeam('ABC123');

      expect(mockPost).toHaveBeenCalledWith('/teams/join', { code: 'ABC123' });
      expect(result).toEqual(team);
    });

    it('should propagate errors for invalid codes', async () => {
      mockPost.mockRejectedValue({ success: false, error: 'Invalid invite code' });

      await expect(teamApi.joinTeam('INVALID')).rejects.toEqual({
        success: false,
        error: 'Invalid invite code',
      });
    });
  });
});
