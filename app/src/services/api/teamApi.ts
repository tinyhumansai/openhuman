import type { Team, TeamInvite, TeamMember, TeamRole, TeamWithRole } from '../../types/team';
import { callCoreRpc } from '../coreRpcClient';

async function rpcResult<T>(method: string, params?: Record<string, unknown>): Promise<T> {
  const response = await callCoreRpc<{ result: T }>({ method, params });
  return response.result;
}

export const teamApi = {
  getTeams: async (): Promise<TeamWithRole[]> => rpcResult('openhuman.team_list_teams'),

  getTeam: async (teamId: string): Promise<Team> =>
    rpcResult('openhuman.team_get_team', { teamId }),

  createTeam: async (name: string): Promise<Team> =>
    rpcResult('openhuman.team_create_team', { name }),

  updateTeam: async (teamId: string, data: { name?: string }): Promise<Team> =>
    rpcResult('openhuman.team_update_team', { teamId, ...data }),

  deleteTeam: async (teamId: string): Promise<void> => {
    await rpcResult('openhuman.team_delete_team', { teamId });
  },

  switchTeam: async (teamId: string): Promise<void> => {
    await rpcResult('openhuman.team_switch_team', { teamId });
  },

  getMembers: async (teamId: string): Promise<TeamMember[]> =>
    rpcResult('openhuman.team_list_members', { teamId }),

  removeMember: async (teamId: string, userId: string): Promise<void> => {
    await rpcResult('openhuman.team_remove_member', { teamId, userId });
  },

  changeMemberRole: async (teamId: string, userId: string, role: TeamRole): Promise<void> => {
    await rpcResult('openhuman.team_change_member_role', { teamId, userId, role });
  },

  leaveTeam: async (teamId: string): Promise<void> => {
    await rpcResult('openhuman.team_leave_team', { teamId });
  },

  createInvite: async (
    teamId: string,
    opts?: { maxUses?: number; expiresInDays?: number }
  ): Promise<TeamInvite> => rpcResult('openhuman.team_create_invite', { teamId, ...opts }),

  getInvites: async (teamId: string): Promise<TeamInvite[]> =>
    rpcResult('openhuman.team_list_invites', { teamId }),

  revokeInvite: async (teamId: string, inviteId: string): Promise<void> => {
    await rpcResult('openhuman.team_revoke_invite', { teamId, inviteId });
  },

  joinTeam: async (code: string): Promise<Team> => rpcResult('openhuman.team_join_team', { code }),
};
