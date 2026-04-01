import type { ApiResponse } from '../../types/api';
import { apiClient } from '../apiClient';

// ── Types ─────────────────────────────────────────────────────────────────────

export interface Tunnel {
  id: string;
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

  /** GET /tunnels/:id — get a specific webhook tunnel */
  getTunnel: async (id: string): Promise<Tunnel> => {
    const response = await apiClient.get<ApiResponse<Tunnel>>(`/tunnels/${id}`);
    return response.data;
  },

  /** PATCH /tunnels/:id — update a webhook tunnel */
  updateTunnel: async (id: string, body: UpdateTunnelRequest): Promise<Tunnel> => {
    const response = await apiClient.patch<ApiResponse<Tunnel>>(`/tunnels/${id}`, body);
    return response.data;
  },

  /** DELETE /tunnels/:id — delete a webhook tunnel */
  deleteTunnel: async (id: string): Promise<void> => {
    await apiClient.delete<ApiResponse<unknown>>(`/tunnels/${id}`);
  },
};
