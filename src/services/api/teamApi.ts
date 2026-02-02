import type { ApiResponse } from '../../types/api';
import type { Team, TeamInvite, TeamMember, TeamRole, TeamWithRole } from '../../types/team';
import { apiClient } from '../apiClient';

export const teamApi = {
  /** GET /teams — list all teams the user belongs to */
  getTeams: async (): Promise<TeamWithRole[]> => {
    const response = await apiClient.get<ApiResponse<TeamWithRole[]>>('/teams');
    return response.data;
  },

  /** GET /teams/:teamId */
  getTeam: async (teamId: string): Promise<Team> => {
    const response = await apiClient.get<ApiResponse<Team>>(`/teams/${teamId}`);
    return response.data;
  },

  /** POST /teams — create a new team */
  createTeam: async (name: string): Promise<Team> => {
    const response = await apiClient.post<ApiResponse<Team>>('/teams', { name });
    return response.data;
  },

  /** PUT /teams/:teamId */
  updateTeam: async (teamId: string, data: { name?: string }): Promise<Team> => {
    const response = await apiClient.put<ApiResponse<Team>>(`/teams/${teamId}`, data);
    return response.data;
  },

  /** DELETE /teams/:teamId */
  deleteTeam: async (teamId: string): Promise<void> => {
    await apiClient.delete<ApiResponse<unknown>>(`/teams/${teamId}`);
  },

  /** POST /teams/:teamId/switch — set as active team */
  switchTeam: async (teamId: string): Promise<void> => {
    await apiClient.post<ApiResponse<unknown>>(`/teams/${teamId}/switch`);
  },

  /** GET /teams/:teamId/members */
  getMembers: async (teamId: string): Promise<TeamMember[]> => {
    const response = await apiClient.get<ApiResponse<TeamMember[]>>(`/teams/${teamId}/members`);
    return response.data;
  },

  /** DELETE /teams/:teamId/members/:userId */
  removeMember: async (teamId: string, userId: string): Promise<void> => {
    await apiClient.delete<ApiResponse<unknown>>(`/teams/${teamId}/members/${userId}`);
  },

  /** PUT /teams/:teamId/members/:userId/role */
  changeMemberRole: async (teamId: string, userId: string, role: TeamRole): Promise<void> => {
    await apiClient.put<ApiResponse<unknown>>(`/teams/${teamId}/members/${userId}/role`, { role });
  },

  /** POST /teams/:teamId/leave */
  leaveTeam: async (teamId: string): Promise<void> => {
    await apiClient.post<ApiResponse<unknown>>(`/teams/${teamId}/leave`);
  },

  /** POST /teams/:teamId/invites */
  createInvite: async (
    teamId: string,
    opts?: { maxUses?: number; expiresInDays?: number }
  ): Promise<TeamInvite> => {
    const response = await apiClient.post<ApiResponse<TeamInvite>>(
      `/teams/${teamId}/invites`,
      opts
    );
    return response.data;
  },

  /** GET /teams/:teamId/invites */
  getInvites: async (teamId: string): Promise<TeamInvite[]> => {
    const response = await apiClient.get<ApiResponse<TeamInvite[]>>(`/teams/${teamId}/invites`);
    return response.data;
  },

  /** DELETE /teams/:teamId/invites/:inviteId */
  revokeInvite: async (teamId: string, inviteId: string): Promise<void> => {
    await apiClient.delete<ApiResponse<unknown>>(`/teams/${teamId}/invites/${inviteId}`);
  },

  /** POST /teams/join — join a team via invite code */
  joinTeam: async (code: string): Promise<Team> => {
    const response = await apiClient.post<ApiResponse<Team>>('/teams/join', { code });
    return response.data;
  },
};
