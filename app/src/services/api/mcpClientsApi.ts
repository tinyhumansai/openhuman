/**
 * Typed RPC wrapper for the MCP Clients domain.
 * All methods call `openhuman.mcp_clients_<function>` and unwrap the
 * `{ result: T }` envelope returned by the core RPC framework.
 *
 * Centralises method-name strings so components never spell them out directly.
 */
import debug from 'debug';

import type {
  ConnStatus,
  InstalledServer,
  McpTool,
  SmitheryServer,
  SmitheryServerDetail,
} from '../../components/channels/mcp/types';
import { callCoreRpc } from '../coreRpcClient';

const log = debug('mcp-clients:api');

// ---------------------------------------------------------------------------
// Response envelopes
// ---------------------------------------------------------------------------

interface RegistrySearchResult {
  servers: SmitheryServer[];
  page: number;
  total_pages: number;
}

interface RegistryGetResult {
  server: SmitheryServerDetail;
}

interface InstalledListResult {
  installed: InstalledServer[];
}

interface InstallResult {
  server: InstalledServer;
}

interface UninstallResult {
  server_id: string;
  removed: boolean;
}

interface ConnectResult {
  server_id: string;
  status: 'connected';
  tools: McpTool[];
}

interface DisconnectResult {
  server_id: string;
  status: 'disconnected';
}

interface StatusResult {
  servers: ConnStatus[];
}

interface ToolCallResult {
  result: unknown;
  is_error: boolean;
}

interface ConfigAssistResult {
  reply: string;
  suggested_env?: Record<string, string>;
}

// ---------------------------------------------------------------------------
// API
// ---------------------------------------------------------------------------

export const mcpClientsApi = {
  /** Search the Smithery registry. Returns paged results. */
  registrySearch: async (params: {
    query?: string;
    page?: number;
    page_size?: number;
  }): Promise<RegistrySearchResult> => {
    log('registry_search params=%o', params);
    const result = await callCoreRpc<RegistrySearchResult>({
      method: 'openhuman.mcp_clients_registry_search',
      params,
    });
    log('registry_search result: %d servers', result.servers?.length ?? 0);
    return result;
  },

  /** Fetch full detail for a single Smithery server. */
  registryGet: async (qualified_name: string): Promise<SmitheryServerDetail> => {
    log('registry_get qualified_name=%s', qualified_name);
    const result = await callCoreRpc<RegistryGetResult>({
      method: 'openhuman.mcp_clients_registry_get',
      params: { qualified_name },
    });
    log('registry_get returned server=%s', result.server?.qualified_name);
    return result.server;
  },

  /** List all locally installed MCP servers. */
  installedList: async (): Promise<InstalledServer[]> => {
    log('installed_list');
    const result = await callCoreRpc<InstalledListResult>({
      method: 'openhuman.mcp_clients_installed_list',
      params: {},
    });
    log(
      'installed_list returned %d servers',
      Array.isArray(result.installed) ? result.installed.length : 0
    );
    // Guard against an unexpected envelope shape (e.g. core returns `{}` on
    // first launch before the MCP store is initialised, or upstream sends a
    // non-array value). Callers downstream call `.find` / `.map` on this
    // array directly — returning anything but an array crashes the MCP
    // Servers tab with `Cannot read properties of undefined (reading 'find')`.
    return Array.isArray(result.installed) ? result.installed : [];
  },

  /** Install a server with the given env vars and optional config. */
  install: async (params: {
    qualified_name: string;
    env: Record<string, string>;
    config?: unknown;
  }): Promise<InstalledServer> => {
    log('install qualified_name=%s', params.qualified_name);
    const result = await callCoreRpc<InstallResult>({
      method: 'openhuman.mcp_clients_install',
      params,
    });
    log('install returned server_id=%s', result.server?.server_id);
    return result.server;
  },

  /** Uninstall a server by ID. */
  uninstall: async (server_id: string): Promise<UninstallResult> => {
    log('uninstall server_id=%s', server_id);
    const result = await callCoreRpc<UninstallResult>({
      method: 'openhuman.mcp_clients_uninstall',
      params: { server_id },
    });
    log('uninstall removed=%s', result.removed);
    return result;
  },

  /** Connect a server and retrieve its available tools. */
  connect: async (server_id: string): Promise<ConnectResult> => {
    log('connect server_id=%s', server_id);
    const result = await callCoreRpc<ConnectResult>({
      method: 'openhuman.mcp_clients_connect',
      params: { server_id },
    });
    log('connect status=%s tools=%d', result.status, result.tools?.length ?? 0);
    return result;
  },

  /** Disconnect a server. */
  disconnect: async (server_id: string): Promise<DisconnectResult> => {
    log('disconnect server_id=%s', server_id);
    const result = await callCoreRpc<DisconnectResult>({
      method: 'openhuman.mcp_clients_disconnect',
      params: { server_id },
    });
    log('disconnect status=%s', result.status);
    return result;
  },

  /** Get status for all managed MCP servers. */
  status: async (): Promise<ConnStatus[]> => {
    log('status');
    const result = await callCoreRpc<StatusResult>({
      method: 'openhuman.mcp_clients_status',
      params: {},
    });
    log('status returned %d servers', Array.isArray(result.servers) ? result.servers.length : 0);
    // Same defensive shape as installedList: downstream `.find` / `.map` callers
    // can't tolerate anything but an array if the RPC envelope is malformed or
    // missing this field.
    return Array.isArray(result.servers) ? result.servers : [];
  },

  /** Invoke a tool on a connected server. */
  toolCall: async (params: {
    server_id: string;
    tool_name: string;
    arguments: unknown;
  }): Promise<ToolCallResult> => {
    log('tool_call server_id=%s tool=%s', params.server_id, params.tool_name);
    const result = await callCoreRpc<ToolCallResult>({
      method: 'openhuman.mcp_clients_tool_call',
      params,
    });
    log('tool_call is_error=%s', result.is_error);
    return result;
  },

  /** Call the LLM-driven configuration assistant. */
  configAssist: async (params: {
    qualified_name: string;
    user_message: string;
    history?: { role: 'user' | 'assistant'; content: string }[];
  }): Promise<ConfigAssistResult> => {
    log('config_assist qualified_name=%s', params.qualified_name);
    const result = await callCoreRpc<ConfigAssistResult>({
      method: 'openhuman.mcp_clients_config_assist',
      params,
    });
    log(
      'config_assist reply length=%d suggested_env=%s',
      result.reply?.length ?? 0,
      result.suggested_env ? 'yes' : 'no'
    );
    return result;
  },
};
