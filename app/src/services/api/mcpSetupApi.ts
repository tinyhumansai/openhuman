/**
 * Typed RPC wrapper for the MCP **setup-agent** domain (`mcp_setup_*`).
 *
 * These RPCs power the agent-native "install MCP server with assistant" flow:
 * search → get → request_secret (out-of-band native prompt) → test_connection →
 * install_and_connect. Secret *values* never travel through here as plaintext
 * the agent sees — `request_secret` returns an opaque `secret://<hex>` ref that
 * the UI fulfils via `submitSecret`, and only refs are passed to
 * `testConnection` / `installAndConnect`.
 *
 * Centralises the `openhuman.mcp_setup_<function>` method-name strings so
 * components never spell them out directly (issue #3039 gap B4).
 */
import debug from 'debug';

import type {
  McpTool,
  SmitheryServer,
  SmitheryServerDetail,
} from '../../components/channels/mcp/types';
import { callCoreRpc } from '../coreRpcClient';

const log = debug('mcp-setup:api');

interface SetupSearchResult {
  servers: SmitheryServer[];
  page: number;
  total_pages: number;
}

interface SetupGetResult {
  server: SmitheryServerDetail;
}

interface RequestSecretResult {
  ref: string;
  key_name: string;
}

interface SubmitSecretResult {
  ref: string;
  fulfilled: boolean;
}

interface TestConnectionResult {
  ok: boolean;
  tools?: McpTool[];
  error?: string;
}

interface InstallAndConnectResult {
  server_id: string;
  status: 'connected' | 'installed_disconnected';
  tools?: McpTool[];
  error?: string;
}

export const mcpSetupApi = {
  /** Search all enabled registries (official modelcontextprotocol.io primary, Smithery fallback). */
  search: async (params: {
    query?: string;
    page?: number;
    page_size?: number;
  }): Promise<SetupSearchResult> => {
    log('search params=%o', params);
    return callCoreRpc<SetupSearchResult>({ method: 'openhuman.mcp_setup_search', params });
  },

  /** Fetch one server's detail with `required_env_keys` injected. */
  get: async (qualified_name: string): Promise<SmitheryServerDetail> => {
    log('get qualified_name=%s', qualified_name);
    const result = await callCoreRpc<SetupGetResult>({
      method: 'openhuman.mcp_setup_get',
      params: { qualified_name },
    });
    return result.server;
  },

  /**
   * Ask the core to request a secret from the user out-of-band. Returns an
   * opaque ref; the value is collected by the native SecretPromptDialog and
   * submitted via {@link submitSecret}.
   */
  requestSecret: async (params: {
    key_name: string;
    prompt: string;
  }): Promise<RequestSecretResult> => {
    log('request_secret key_name=%s', params.key_name);
    return callCoreRpc<RequestSecretResult>({
      method: 'openhuman.mcp_setup_request_secret',
      params,
    });
  },

  /** UI-side: fulfil a pending `request_secret` with the user-entered value. */
  submitSecret: async (params: { ref_id: string; value: string }): Promise<SubmitSecretResult> => {
    // Intentionally NOT logging the value.
    log('submit_secret ref_id=%s', params.ref_id);
    return callCoreRpc<SubmitSecretResult>({ method: 'openhuman.mcp_setup_submit_secret', params });
  },

  /** Dry-run a candidate install (spawn, list tools, tear down — nothing persisted). */
  testConnection: async (params: {
    qualified_name: string;
    env_refs: Record<string, string>;
  }): Promise<TestConnectionResult> => {
    log('test_connection qualified_name=%s', params.qualified_name);
    const result = await callCoreRpc<TestConnectionResult>({
      method: 'openhuman.mcp_setup_test_connection',
      params,
    });
    log('test_connection ok=%s', result.ok);
    return result;
  },

  /** Commit the install + secrets, then connect and return the tool list. */
  installAndConnect: async (params: {
    qualified_name: string;
    env_refs: Record<string, string>;
  }): Promise<InstallAndConnectResult> => {
    log('install_and_connect qualified_name=%s', params.qualified_name);
    const result = await callCoreRpc<InstallAndConnectResult>({
      method: 'openhuman.mcp_setup_install_and_connect',
      params,
    });
    log('install_and_connect status=%s', result.status);
    return result;
  },
};
