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

// ── Read operations ───────────────────────────────────────────────

export async function listToolkits(): Promise<ComposioToolkitsResponse> {
  return callCoreRpc<ComposioToolkitsResponse>({
    method: 'openhuman.composio_list_toolkits',
  });
}

export async function listConnections(): Promise<ComposioConnectionsResponse> {
  return callCoreRpc<ComposioConnectionsResponse>({
    method: 'openhuman.composio_list_connections',
  });
}

export async function listTools(
  toolkits?: string[],
): Promise<ComposioToolsResponse> {
  return callCoreRpc<ComposioToolsResponse>({
    method: 'openhuman.composio_list_tools',
    params: toolkits && toolkits.length > 0 ? { toolkits } : {},
  });
}

// ── Write operations ──────────────────────────────────────────────

/**
 * Begin an OAuth handoff for `toolkit`. The returned `connectUrl`
 * must be opened in a browser for the user to complete the flow.
 * The core publishes a `ComposioConnectionCreated` event on success.
 */
export async function authorize(
  toolkit: string,
): Promise<ComposioAuthorizeResponse> {
  return callCoreRpc<ComposioAuthorizeResponse>({
    method: 'openhuman.composio_authorize',
    params: { toolkit },
  });
}

/**
 * Delete an existing Composio connection. Backend verifies ownership
 * before forwarding to Composio.
 */
export async function deleteConnection(
  connectionId: string,
): Promise<ComposioDeleteResponse> {
  return callCoreRpc<ComposioDeleteResponse>({
    method: 'openhuman.composio_delete_connection',
    params: { connection_id: connectionId },
  });
}

/**
 * Execute a Composio action slug (e.g. `GMAIL_SEND_EMAIL`). The core
 * charges the caller, tracks usage, and publishes a
 * `ComposioActionExecuted` event.
 */
export async function execute(
  tool: string,
  args?: Record<string, unknown>,
): Promise<ComposioExecuteResponse> {
  return callCoreRpc<ComposioExecuteResponse>({
    method: 'openhuman.composio_execute',
    params: { tool, arguments: args ?? {} },
  });
}
