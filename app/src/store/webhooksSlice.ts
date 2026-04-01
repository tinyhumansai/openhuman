import { createSlice, type PayloadAction } from '@reduxjs/toolkit';

import type { Tunnel } from '../services/api/tunnelsApi';

// ── Types ─────────────────────────────────────────────────────────────────────

/** Local tunnel-to-skill registration (from the Rust core WebhookRouter). */
export interface TunnelRegistration {
  tunnel_uuid: string;
  skill_id: string;
  tunnel_name: string | null;
  backend_tunnel_id: string | null;
}

/** Entry in the webhook activity log. */
export interface WebhookActivityEntry {
  correlation_id: string;
  tunnel_name: string;
  method: string;
  path: string;
  status_code: number | null;
  skill_id: string | null;
  timestamp: number;
}

interface WebhooksState {
  /** Tunnels from the backend API. */
  tunnels: Tunnel[];
  /** Local tunnel-to-skill registrations from the Rust core. */
  registrations: TunnelRegistration[];
  /** Recent webhook activity (ring buffer, newest first). */
  activity: WebhookActivityEntry[];
  loading: boolean;
  error: string | null;
}

// ── Slice ─────────────────────────────────────────────────────────────────────

const MAX_ACTIVITY_ENTRIES = 100;

const initialState: WebhooksState = {
  tunnels: [],
  registrations: [],
  activity: [],
  loading: false,
  error: null,
};

const webhooksSlice = createSlice({
  name: 'webhooks',
  initialState,
  reducers: {
    setTunnels: (state, action: PayloadAction<Tunnel[]>) => {
      state.tunnels = action.payload;
      state.loading = false;
      state.error = null;
    },
    addTunnel: (state, action: PayloadAction<Tunnel>) => {
      state.tunnels.push(action.payload);
    },
    removeTunnel: (state, action: PayloadAction<string>) => {
      state.tunnels = state.tunnels.filter(t => t.id !== action.payload);
    },
    setRegistrations: (state, action: PayloadAction<TunnelRegistration[]>) => {
      state.registrations = action.payload;
    },
    addActivity: (state, action: PayloadAction<WebhookActivityEntry>) => {
      state.activity.unshift(action.payload);
      if (state.activity.length > MAX_ACTIVITY_ENTRIES) {
        state.activity.splice(MAX_ACTIVITY_ENTRIES);
      }
    },
    setLoading: (state, action: PayloadAction<boolean>) => {
      state.loading = action.payload;
    },
    setError: (state, action: PayloadAction<string | null>) => {
      state.error = action.payload;
      state.loading = false;
    },
  },
});

export const {
  setTunnels,
  addTunnel,
  removeTunnel,
  setRegistrations,
  addActivity,
  setLoading,
  setError,
} = webhooksSlice.actions;
export default webhooksSlice.reducer;
