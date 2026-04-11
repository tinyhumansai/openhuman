/**
 * TypeScript types that mirror the Rust `openhuman::composio::types`
 * response envelopes exposed via the `openhuman.composio_*` JSON-RPC
 * methods. Field names match the wire shape (camelCase where the
 * backend emits camelCase, snake_case where the Rust RPC layer does).
 */

export interface ComposioToolkitsResponse {
  toolkits: string[];
}

export interface ComposioConnection {
  id: string;
  toolkit: string;
  /** Typical values: `ACTIVE`, `CONNECTED`, `PENDING`, `FAILED`. */
  status: string;
  /** ISO timestamp (backend passthrough). */
  createdAt?: string;
}

export interface ComposioConnectionsResponse {
  connections: ComposioConnection[];
}

export interface ComposioAuthorizeResponse {
  /** Composio-hosted OAuth URL that must be opened in a browser. */
  connectUrl: string;
  /** New Composio connection id created by the authorize call. */
  connectionId: string;
}

export interface ComposioDeleteResponse {
  deleted: boolean;
}

export interface ComposioToolFunction {
  name: string;
  description?: string;
  parameters?: Record<string, unknown>;
}

export interface ComposioToolSchema {
  /** Usually the literal string `"function"`. */
  type: string;
  function: ComposioToolFunction;
}

export interface ComposioToolsResponse {
  tools: ComposioToolSchema[];
}

export interface ComposioExecuteResponse {
  data: unknown;
  successful: boolean;
  error?: string | null;
  costUsd: number;
}

// ── UI helpers ────────────────────────────────────────────────────

/**
 * Derived connection state used by the Skills grid card.
 * Mirrors the `SkillConnectionStatus` shape so the same
 * `UnifiedSkillCard` can render both.
 */
export type ComposioConnectionState = 'disconnected' | 'pending' | 'connected' | 'error';

export function deriveComposioState(
  connection: ComposioConnection | undefined
): ComposioConnectionState {
  if (!connection) return 'disconnected';
  const status = connection.status.toUpperCase();
  if (status === 'ACTIVE' || status === 'CONNECTED') return 'connected';
  if (status === 'PENDING' || status === 'INITIATED' || status === 'INITIALIZING') return 'pending';
  if (status === 'FAILED' || status === 'ERROR' || status === 'EXPIRED') return 'error';
  return 'disconnected';
}
