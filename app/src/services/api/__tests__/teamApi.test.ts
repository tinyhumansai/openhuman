import { beforeEach, describe, expect, it, vi } from 'vitest';

import { teamApi } from '../teamApi';

const mockCallCoreRpc = vi.fn();

vi.mock('../../coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mockCallCoreRpc(...args),
}));

describe('teamApi', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  describe('getTeams', () => {
    it('calls team_list_teams and returns the result', async () => {
      const teams = [{ team: { _id: 't1', name: 'Team 1' }, role: 'ADMIN' }];
      mockCallCoreRpc.mockResolvedValue({ result: teams });

      const result = await teamApi.getTeams();

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.team_list_teams',
        params: undefined,
      });
      expect(result).toEqual(teams);
    });

    it('propagates RPC errors', async () => {
      const error = new Error('Unauthorized');
      mockCallCoreRpc.mockRejectedValue(error);

      await expect(teamApi.getTeams()).rejects.toThrow('Unauthorized');
    });
  });

  describe('getTeam', () => {
    it('calls team_get_team', async () => {
      const team = { _id: 't1', name: 'Team 1' };
      mockCallCoreRpc.mockResolvedValue({ result: team });

      const result = await teamApi.getTeam('t1');

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.team_get_team',
        params: { teamId: 't1' },
      });
      expect(result).toEqual(team);
    });
  });

  describe('createTeam', () => {
    it('calls team_create_team with name', async () => {
      const team = { _id: 't2', name: 'New Team' };
      mockCallCoreRpc.mockResolvedValue({ result: team });

      const result = await teamApi.createTeam('New Team');

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.team_create_team',
        params: { name: 'New Team' },
      });
      expect(result).toEqual(team);
    });
  });

  describe('updateTeam', () => {
    it('calls team_update_team with data', async () => {
      const team = { _id: 't1', name: 'Updated' };
      mockCallCoreRpc.mockResolvedValue({ result: team });

      const result = await teamApi.updateTeam('t1', { name: 'Updated' });

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.team_update_team',
        params: { teamId: 't1', name: 'Updated' },
      });
      expect(result).toEqual(team);
    });
  });

  describe('deleteTeam', () => {
    it('calls team_delete_team', async () => {
      mockCallCoreRpc.mockResolvedValue({ result: null });

      await teamApi.deleteTeam('t1');

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.team_delete_team',
        params: { teamId: 't1' },
      });
    });
  });

  describe('switchTeam', () => {
    it('calls team_switch_team', async () => {
      mockCallCoreRpc.mockResolvedValue({ result: null });

      await teamApi.switchTeam('t1');

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.team_switch_team',
        params: { teamId: 't1' },
      });
    });
  });

  describe('getMembers', () => {
    it('calls team_list_members', async () => {
      const members = [
        {
          _id: 'm1',
          user: { _id: 'u1', firstName: 'John' },
          role: 'ADMIN',
          joinedAt: '2026-01-01',
        },
      ];
      mockCallCoreRpc.mockResolvedValue({ result: members });

      const result = await teamApi.getMembers('t1');

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.team_list_members',
        params: { teamId: 't1' },
      });
      expect(result).toEqual(members);
    });
  });

  describe('removeMember', () => {
    it('calls team_remove_member', async () => {
      mockCallCoreRpc.mockResolvedValue({ result: null });

      await teamApi.removeMember('t1', 'u2');

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.team_remove_member',
        params: { teamId: 't1', userId: 'u2' },
      });
    });
  });

  describe('changeMemberRole', () => {
    it('calls team_change_member_role', async () => {
      mockCallCoreRpc.mockResolvedValue({ result: null });

      await teamApi.changeMemberRole('t1', 'u2', 'BILLING_MANAGER');

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.team_change_member_role',
        params: { teamId: 't1', userId: 'u2', role: 'BILLING_MANAGER' },
      });
    });
  });

  describe('leaveTeam', () => {
    it('calls team_leave_team', async () => {
      mockCallCoreRpc.mockResolvedValue({ result: null });

      await teamApi.leaveTeam('t1');

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.team_leave_team',
        params: { teamId: 't1' },
      });
    });
  });

  describe('createInvite', () => {
    it('calls team_create_invite without optional fields by default', async () => {
      const invite = { _id: 'inv1', code: 'ABC123' };
      mockCallCoreRpc.mockResolvedValue({ result: invite });

      const result = await teamApi.createInvite('t1');

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.team_create_invite',
        params: { teamId: 't1' },
      });
      expect(result).toEqual(invite);
    });

    it('passes invite options when provided', async () => {
      const invite = { _id: 'inv2', code: 'XYZ789' };
      mockCallCoreRpc.mockResolvedValue({ result: invite });

      await teamApi.createInvite('t1', { maxUses: 5, expiresInDays: 7 });

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.team_create_invite',
        params: { teamId: 't1', maxUses: 5, expiresInDays: 7 },
      });
    });
  });

  describe('getInvites', () => {
    it('calls team_list_invites', async () => {
      const invites = [{ _id: 'inv1', code: 'ABC123' }];
      mockCallCoreRpc.mockResolvedValue({ result: invites });

      const result = await teamApi.getInvites('t1');

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.team_list_invites',
        params: { teamId: 't1' },
      });
      expect(result).toEqual(invites);
    });
  });

  describe('revokeInvite', () => {
    it('calls team_revoke_invite', async () => {
      mockCallCoreRpc.mockResolvedValue({ result: null });

      await teamApi.revokeInvite('t1', 'inv1');

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.team_revoke_invite',
        params: { teamId: 't1', inviteId: 'inv1' },
      });
    });
  });

  describe('joinTeam', () => {
    it('calls team_join_team with code', async () => {
      const team = { _id: 't3', name: 'Joined Team' };
      mockCallCoreRpc.mockResolvedValue({ result: team });

      const result = await teamApi.joinTeam('ABC123');

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.team_join_team',
        params: { code: 'ABC123' },
      });
      expect(result).toEqual(team);
    });

    it('propagates errors for invalid codes', async () => {
      const error = new Error('Invalid invite code');
      mockCallCoreRpc.mockRejectedValue(error);

      await expect(teamApi.joinTeam('INVALID')).rejects.toThrow('Invalid invite code');
    });
  });
});
