/**
 * Imperative RPC wrapper for the Composio domain — typed counterpart
 * to `src/openhuman/composio/*` on the Rust side.
 *
 * Every function here calls the core sidecar via JSON-RPC. The core
 * in turn proxies to the openhuman backend's
 * `/agent-integrations/composio/*` routes, so the frontend never talks
 * to Composio directly and never handles the API key.
 *
 * This mirrors the shape of `lib/skills/skillsApi.ts`. Keep the two
 * files stylistically consistent so the parallel domain stays easy to
 * grok.
 */
import { callCoreRpc } from '../../services/coreRpcClient';
import type {
  ComposioAuthorizeResponse,
  ComposioConnectionsResponse,
  ComposioDeleteResponse,
  ComposioExecuteResponse,
  ComposioToolkitsResponse,
  ComposioToolsResponse,
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
