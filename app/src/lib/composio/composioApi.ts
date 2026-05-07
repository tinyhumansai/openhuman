/**
 * Imperative RPC wrapper for the Composio domain — typed counterpart
 * to `src/openhuman/composio/*` on the Rust side.
 *
 * Every function here calls the core sidecar via JSON-RPC. The core
 * in turn proxies to the openhuman backend's
 * `/agent-integrations/composio/*` routes, so the frontend never talks
 * to Composio directly and never handles the API key.
 *
 * Keep this file stylistically consistent with the other RPC wrappers
 * in `app/src/utils/tauriCommands` so the domain stays easy to grok.
 */
import { callCoreRpc } from '../../services/coreRpcClient';
import type {
  ComposioActiveTriggersResponse,
  ComposioAuthorizeResponse,
  ComposioAvailableTriggersResponse,
  ComposioConnectionsResponse,
  ComposioDeleteResponse,
  ComposioDisableTriggerResponse,
  ComposioEnableTriggerResponse,
  ComposioExecuteResponse,
  ComposioToolkitsResponse,
  ComposioToolsResponse,
  ComposioUserScopePref,
} from './types';

/**
 * Every `composio_*` op on the Rust side returns an `RpcOutcome` with a
 * user-visible log line attached. `RpcOutcome::into_cli_compatible_json`
 * (see `src/rpc/mod.rs`) therefore wraps the payload as
 * `{ "result": <flat shape>, "logs": [...] }` before handing it to the
 * JSON-RPC layer. This helper peels that envelope back off so every
 * caller in this file can work with the flat shapes declared in
 * `./types`. Responses without logs pass through unchanged.
 */
function unwrapCliEnvelope<T>(value: unknown): T {
  if (
    value !== null &&
    typeof value === 'object' &&
    'result' in (value as Record<string, unknown>) &&
    'logs' in (value as Record<string, unknown>) &&
    Array.isArray((value as { logs: unknown }).logs)
  ) {
    return (value as { result: T }).result;
  }
  return value as T;
}

// ── Read operations ───────────────────────────────────────────────

export async function listToolkits(): Promise<ComposioToolkitsResponse> {
  const raw = await callCoreRpc<unknown>({ method: 'openhuman.composio_list_toolkits' });
  return unwrapCliEnvelope<ComposioToolkitsResponse>(raw);
}

export async function listConnections(): Promise<ComposioConnectionsResponse> {
  const raw = await callCoreRpc<unknown>({ method: 'openhuman.composio_list_connections' });
  return unwrapCliEnvelope<ComposioConnectionsResponse>(raw);
}

export async function listTools(toolkits?: string[]): Promise<ComposioToolsResponse> {
  const raw = await callCoreRpc<unknown>({
    method: 'openhuman.composio_list_tools',
    params: toolkits && toolkits.length > 0 ? { toolkits } : {},
  });
  return unwrapCliEnvelope<ComposioToolsResponse>(raw);
}

// ── Write operations ──────────────────────────────────────────────

/**
 * Begin an OAuth handoff for `toolkit`. The returned `connectUrl`
 * must be opened in a browser for the user to complete the flow.
 * The core publishes a `ComposioConnectionCreated` event on success.
 */
export async function authorize(toolkit: string): Promise<ComposioAuthorizeResponse> {
  const raw = await callCoreRpc<unknown>({
    method: 'openhuman.composio_authorize',
    params: { toolkit },
  });
  return unwrapCliEnvelope<ComposioAuthorizeResponse>(raw);
}

/**
 * Delete an existing Composio connection. Backend verifies ownership
 * before forwarding to Composio.
 */
export async function deleteConnection(connectionId: string): Promise<ComposioDeleteResponse> {
  const raw = await callCoreRpc<unknown>({
    method: 'openhuman.composio_delete_connection',
    params: { connection_id: connectionId },
  });
  return unwrapCliEnvelope<ComposioDeleteResponse>(raw);
}

/**
 * Read the per-toolkit user scope preference (read/write/admin) used
 * to gate `composio_execute`. Returns the default
 * `{ read: true, write: true, admin: false }` when nothing is stored.
 */
export async function getUserScopes(toolkit: string): Promise<ComposioUserScopePref> {
  console.debug('[composio][scopes] → openhuman.composio_get_user_scopes toolkit=%s', toolkit);
  const raw = await callCoreRpc<unknown>({
    method: 'openhuman.composio_get_user_scopes',
    params: { toolkit },
  });
  const pref = unwrapCliEnvelope<ComposioUserScopePref>(raw);
  console.debug(
    '[composio][scopes] ← openhuman.composio_get_user_scopes toolkit=%s pref=%o',
    toolkit,
    pref
  );
  return pref;
}

/**
 * Persist a per-toolkit user scope preference. The agent will only be
 * able to invoke composio actions whose classified scope is enabled
 * here.
 */
export async function setUserScopes(
  toolkit: string,
  pref: ComposioUserScopePref
): Promise<ComposioUserScopePref> {
  console.debug(
    '[composio][scopes] → openhuman.composio_set_user_scopes toolkit=%s pref=%o',
    toolkit,
    pref
  );
  const raw = await callCoreRpc<unknown>({
    method: 'openhuman.composio_set_user_scopes',
    params: { toolkit, ...pref },
  });
  const persisted = unwrapCliEnvelope<ComposioUserScopePref>(raw);
  console.debug(
    '[composio][scopes] ← openhuman.composio_set_user_scopes toolkit=%s persisted=%o',
    toolkit,
    persisted
  );
  return persisted;
}

/**
 * Execute a Composio action slug (e.g. `GMAIL_SEND_EMAIL`). The core
 * charges the caller, tracks usage, and publishes a
 * `ComposioActionExecuted` event.
 */
export async function execute(
  tool: string,
  args?: Record<string, unknown>
): Promise<ComposioExecuteResponse> {
  const raw = await callCoreRpc<unknown>({
    method: 'openhuman.composio_execute',
    params: { tool, arguments: args ?? {} },
  });
  return unwrapCliEnvelope<ComposioExecuteResponse>(raw);
}

/**
 * Run a sync pass for a Composio connection by dispatching to the
 * toolkit's native provider implementation (Gmail, Slack, Notion, …).
 * Persists the fetched items into the memory layer — chunks land in
 * `mem_tree_chunks` and the source-tree pipeline picks them up on the
 * next flush. Wraps `openhuman.composio_sync`.
 *
 * `reason` defaults to `"manual"` server-side when omitted.
 */
export async function syncConnection(
  connectionId: string,
  reason: 'manual' | 'periodic' | 'connection_created' = 'manual'
): Promise<unknown> {
  console.debug(
    '[composio][sync] → openhuman.composio_sync conn=%s reason=%s',
    connectionId,
    reason
  );
  const raw = await callCoreRpc<unknown>({
    method: 'openhuman.composio_sync',
    params: { connection_id: connectionId, reason },
  });
  const outcome = unwrapCliEnvelope<unknown>(raw);
  // Avoid logging the raw outcome — provider sync responses can carry
  // message-level PII (subjects, sender addresses, body excerpts).
  // Surface a sanitised shape (top-level keys + payload type) instead.
  const outcomeShape =
    outcome && typeof outcome === 'object'
      ? { keys: Object.keys(outcome as Record<string, unknown>).slice(0, 10) }
      : { type: typeof outcome };
  console.debug(
    '[composio][sync] ← openhuman.composio_sync conn=%s outcome_shape=%o',
    connectionId,
    outcomeShape
  );
  return outcome;
}

// ── Trigger management ────────────────────────────────────────────

/**
 * List the catalog of triggers the user could enable for a toolkit.
 * For GitHub, the backend fans out into per-repo entries — pass the
 * GitHub `connectionId` (or the user's first GitHub connection is
 * picked by the backend).
 */
export async function listAvailableTriggers(
  toolkit: string,
  connectionId?: string
): Promise<ComposioAvailableTriggersResponse> {
  const params: Record<string, unknown> = { toolkit };
  if (connectionId) params.connection_id = connectionId;
  const raw = await callCoreRpc<unknown>({
    method: 'openhuman.composio_list_available_triggers',
    params,
  });
  return unwrapCliEnvelope<ComposioAvailableTriggersResponse>(raw);
}

/**
 * List the user's currently enabled Composio triggers.
 */
export async function listTriggers(toolkit?: string): Promise<ComposioActiveTriggersResponse> {
  const params: Record<string, unknown> = {};
  if (toolkit) params.toolkit = toolkit;
  const raw = await callCoreRpc<unknown>({ method: 'openhuman.composio_list_triggers', params });
  return unwrapCliEnvelope<ComposioActiveTriggersResponse>(raw);
}

/**
 * Enable a single trigger on a connection the caller owns.
 */
export async function enableTrigger(
  connectionId: string,
  slug: string,
  triggerConfig?: Record<string, unknown>
): Promise<ComposioEnableTriggerResponse> {
  const params: Record<string, unknown> = { connection_id: connectionId, slug };
  if (triggerConfig !== undefined) params.trigger_config = triggerConfig;
  const raw = await callCoreRpc<unknown>({ method: 'openhuman.composio_enable_trigger', params });
  return unwrapCliEnvelope<ComposioEnableTriggerResponse>(raw);
}

/**
 * Disable (delete) a trigger owned by the caller.
 */
export async function disableTrigger(triggerId: string): Promise<ComposioDisableTriggerResponse> {
  const raw = await callCoreRpc<unknown>({
    method: 'openhuman.composio_disable_trigger',
    params: { trigger_id: triggerId },
  });
  return unwrapCliEnvelope<ComposioDisableTriggerResponse>(raw);
}
