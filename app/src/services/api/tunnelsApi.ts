import type { ApiResponse } from '../../types/api';
import { apiClient } from '../apiClient';

// ── Types ─────────────────────────────────────────────────────────────────────

export interface Tunnel {
  /** Internal backend ID (used for CRUD endpoints: GET/PATCH/DELETE /tunnels/:id). */
  id: string;
  /** External UUID used for webhook routing (appears in webhook URLs and local registrations). */
  uuid: string;
  name: string;
  description?: string;
  isActive: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface TunnelBandwidthUsage {
  usedBytes: number;
  limitBytes: number;
  cycleStartDate: string;
  cycleEndDate: string;
}

export interface CreateTunnelRequest {
  name: string;
  description?: string;
}

export interface UpdateTunnelRequest {
  name?: string;
  description?: string;
  isActive?: boolean;
}

// ── API ───────────────────────────────────────────────────────────────────────

export const tunnelsApi = {
  /** POST /tunnels — create a new webhook tunnel */
  createTunnel: async (body: CreateTunnelRequest): Promise<Tunnel> => {
    const response = await apiClient.post<ApiResponse<Tunnel>>('/tunnels', body);
    return response.data;
  },

  /** GET /tunnels — list user's webhook tunnels */
  getTunnels: async (): Promise<Tunnel[]> => {
    const response = await apiClient.get<ApiResponse<Tunnel[]>>('/tunnels');
    return response.data;
  },

  /** GET /tunnels/bandwidth — get bandwidth usage for current billing cycle */
  getBandwidthUsage: async (): Promise<TunnelBandwidthUsage> => {
    const response = await apiClient.get<ApiResponse<TunnelBandwidthUsage>>('/tunnels/bandwidth');
    return response.data;
  },

  /** GET /tunnels/:tunnelId — get a specific webhook tunnel by its internal ID. */
  getTunnel: async (tunnelId: string): Promise<Tunnel> => {
    const response = await apiClient.get<ApiResponse<Tunnel>>(`/tunnels/${tunnelId}`);
    return response.data;
  },

  /** PATCH /tunnels/:tunnelId — update a webhook tunnel by its internal ID. */
  updateTunnel: async (tunnelId: string, body: UpdateTunnelRequest): Promise<Tunnel> => {
    const response = await apiClient.patch<ApiResponse<Tunnel>>(`/tunnels/${tunnelId}`, body);
    return response.data;
  },

  /** DELETE /tunnels/:tunnelId — delete a webhook tunnel by its internal ID. */
  deleteTunnel: async (tunnelId: string): Promise<void> => {
    await apiClient.delete<ApiResponse<unknown>>(`/tunnels/${tunnelId}`);
  },
};
