import { callCoreCommand } from '../coreCommandClient';

// const WEBHOOKS_CORE_BASE = '/webhooks/core';
const WEBHOOKS_INGRESS_BASE = '/webhooks/ingress';

// ── Types ─────────────────────────────────────────────────────────────────────

export interface Tunnel {
  /** Internal backend ID (used for CRUD endpoints: GET/PATCH/DELETE /webhooks/core/:id). */
  id: string;
  /** External UUID used for ingress routing (appears in webhook URLs and local registrations). */
  uuid: string;
  name: string;
  description?: string;
  isActive: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface TunnelBandwidthUsage {
  remainingBudgetUsd: number;
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
  /** POST /webhooks/core — create a new webhook tunnel */
  createTunnel: async (body: CreateTunnelRequest): Promise<Tunnel> => {
    return await callCoreCommand<Tunnel>('openhuman.webhooks_create_tunnel', body);
  },

  /** GET /webhooks/core — list user's webhook tunnels */
  getTunnels: async (): Promise<Tunnel[]> => {
    return await callCoreCommand<Tunnel[]>('openhuman.webhooks_list_tunnels');
  },

  /** GET /webhooks/core/bandwidth — get remaining webhook bandwidth budget */
  getBandwidthUsage: async (): Promise<TunnelBandwidthUsage> => {
    return await callCoreCommand<TunnelBandwidthUsage>('openhuman.webhooks_get_bandwidth');
  },

  /** GET /webhooks/core/:tunnelId — get a specific webhook tunnel by its internal ID. */
  getTunnel: async (tunnelId: string): Promise<Tunnel> => {
    return await callCoreCommand<Tunnel>('openhuman.webhooks_get_tunnel', { id: tunnelId });
  },

  /** PATCH /webhooks/core/:tunnelId — update a webhook tunnel by its internal ID. */
  updateTunnel: async (tunnelId: string, body: UpdateTunnelRequest): Promise<Tunnel> => {
    return await callCoreCommand<Tunnel>('openhuman.webhooks_update_tunnel', {
      id: tunnelId,
      ...body,
    });
  },

  /** DELETE /webhooks/core/:tunnelId — delete a webhook tunnel by its internal ID. */
  deleteTunnel: async (tunnelId: string): Promise<void> => {
    await callCoreCommand<unknown>('openhuman.webhooks_delete_tunnel', { id: tunnelId });
  },

  ingressUrl: (backendUrl: string, tunnelUuid: string): string =>
    `${backendUrl.replace(/\/$/, '')}${WEBHOOKS_INGRESS_BASE}/${tunnelUuid}`,
};
