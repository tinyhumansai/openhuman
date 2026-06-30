/**
 * TypeScript types that mirror the Rust `openhuman::composio::types`
 * response envelopes exposed via the `openhuman.composio_*` JSON-RPC
 * methods. Field names match the wire shape (camelCase where the
 * backend emits camelCase, snake_case where the Rust RPC layer does).
 */

/**
 * One toolkit as described by the live Composio catalog. The backend
 * fetches this from `composio.toolkits.get({})` (see the workspace
 * `COMPOSIO_DYNAMIC_CATALOG_PLAN.md`), caches it, and forwards it through
 * the core so the desktop UI no longer has to hardcode display metadata.
 *
 * Everything except `slug` is best-effort: older cores/backends that
 * predate the dynamic catalog omit `catalog` entirely, so consumers must
 * fall back to the local `toolkitMeta` derivation.
 */
export interface ComposioToolkitCatalogEntry {
  /** Toolkit slug as Composio emits it, e.g. `"googlecalendar"`. */
  slug: string;
  /** Human-readable name, e.g. `"Google Calendar"`. */
  name: string;
  /** Composio-hosted logo URL (`meta.logo`). */
  logo?: string;
  /** Short description (`meta.description`). */
  description?: string;
  /** Composio category names (`meta.categories[].name`/slug). */
  categories?: string[];
  /**
   * Whether the user can actually connect/use this toolkit right now —
   * i.e. it passed the backend curation/allowlist gate. Uncurated
   * toolkits still appear in the catalog but render as preview.
   */
  enabled?: boolean;
}

export interface ComposioToolkitsResponse {
  /** Back-compat: slugs the user is allowed to connect (enabled only). */
  toolkits: string[];
  /**
   * Rich render model from the dynamic Composio catalog. Optional —
   * cores that predate the dynamic catalog send only `toolkits`.
   */
  catalog?: ComposioToolkitCatalogEntry[];
}

/**
 * Sorted list of toolkit slugs that ship a curated agent-ready
 * catalog on the core side. Used by the Skills grid to label
 * connected-but-uncurated toolkits as preview / coming soon so
 * users don't trigger the max-iterations failure documented in
 * issue #2283.
 */
export interface ComposioAgentReadyToolkitsResponse {
  toolkits: string[];
}

export interface ComposioConnection {
  id: string;
  toolkit: string;
  /** Typical values: `ACTIVE`, `CONNECTED`, `PENDING`, `FAILED`, `EXPIRED`. */
  status: string;
  /** ISO timestamp (backend passthrough). */
  createdAt?: string;

  /** Optional friendly identity fields populated by later backend versions. */
  accountEmail?: string;
  workspace?: string;
  username?: string;
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
  memory_chunks_deleted?: number;
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

/**
 * Per-toolkit scope preference stored in the core's KV. Default is
 * `{ read: true, write: true, admin: false }`.
 */
export interface ComposioUserScopePref {
  read: boolean;
  write: boolean;
  admin: boolean;
}

// ── GitHub repos ──────────────────────────────────────────────────

export interface ComposioGithubRepo {
  owner: string;
  repo: string;
  fullName: string;
  private?: boolean;
  defaultBranch?: string;
  htmlUrl?: string;
}

export interface ComposioGithubReposResponse {
  connectionId: string;
  repositories: ComposioGithubRepo[];
}

// ── Trigger management ─────────────────────────────────────────────

export type ComposioAvailableTriggerScope = 'static' | 'github_repo';

export interface ComposioAvailableTrigger {
  slug: string;
  scope: ComposioAvailableTriggerScope;
  defaultConfig?: Record<string, unknown>;
  requiredConfigKeys?: string[];
  repo?: { owner: string; repo: string };
}

export interface ComposioAvailableTriggersResponse {
  triggers: ComposioAvailableTrigger[];
}

export interface ComposioActiveTrigger {
  id: string;
  slug: string;
  toolkit: string;
  connectionId: string;
  triggerConfig?: Record<string, unknown>;
  state?: string;
}

export interface ComposioActiveTriggersResponse {
  triggers: ComposioActiveTrigger[];
}

export interface ComposioEnableTriggerResponse {
  triggerId: string;
  slug: string;
  connectionId: string;
}

export interface ComposioDisableTriggerResponse {
  deleted: boolean;
}

// ── UI helpers ────────────────────────────────────────────────────

/**
 * Derived connection state used by the Skills grid card.
 * Mirrors the `SkillConnectionStatus` shape so the same
 * `UnifiedSkillCard` can render both.
 */
export type ComposioConnectionState =
  | 'disconnected'
  | 'pending'
  | 'connected'
  | 'expired'
  | 'error';

export function deriveComposioState(
  connection: ComposioConnection | undefined
): ComposioConnectionState {
  if (!connection) return 'disconnected';
  const status = connection.status.toUpperCase();
  if (status === 'ACTIVE' || status === 'CONNECTED') return 'connected';
  if (status === 'PENDING' || status === 'INITIATED' || status === 'INITIALIZING') return 'pending';
  if (status === 'EXPIRED') return 'expired';
  if (status === 'FAILED' || status === 'ERROR') return 'error';
  return 'disconnected';
}

export interface ComposioConnectionsState {
  primary: ComposioConnectionState;
  count: number;
}

/**
 * Derive composite state from multiple connections for a toolkit.
 * Uses the first connection's state as primary (caller must ensure
 * connections are sorted by priority/age), and reports the total count.
 */
export function deriveComposioStates(
  connections: ComposioConnection[] | undefined
): ComposioConnectionsState {
  if (!connections || connections.length === 0) return { primary: 'disconnected', count: 0 };
  return { primary: deriveComposioState(connections[0]), count: connections.length };
}
