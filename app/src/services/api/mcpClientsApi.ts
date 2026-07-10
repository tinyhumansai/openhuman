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

interface SetEnabledResult {
  server_id: string;
  enabled: boolean;
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

interface UpdateEnvResult {
  server_id: string;
  status: 'connected' | 'disconnected' | 'disabled' | 'unauthorized';
  env_keys: string[];
  tools?: McpTool[];
  error?: string;
  /**
   * Stable auth-failure reason code present when `status === 'unauthorized'`:
   * `'oauth_required'` (use Sign in — a pasted token won't work),
   * `'token_rejected'` (credential sent but refused), or
   * `'credential_required'` (auth needed, none provided). The raw 401 message
   * is intentionally withheld server-side (it leaks the OAuth metadata URL),
   * so the UI maps this code to localized copy (#4289).
   */
  auth_hint?: string;
}

/** Non-secret registry-credentials snapshot. Secret *values* are never returned. */
export interface RegistrySettings {
  smithery_api_key_set: boolean;
  mcp_official_token_set: boolean;
  mcp_official_base?: string | null;
}

// ---------------------------------------------------------------------------
// API
// ---------------------------------------------------------------------------

export const mcpClientsApi = {
  /** Search the Smithery registry. Returns paged results. */
  registrySearch: async (params: {
    query?: string;
    /** Transport filter: 'stdio' | 'hosted' | 'all' (omit for all). */
    transport?: string;
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

  /**
   * Probe how an installed server authenticates so the connect modal can show
   * the right control: `none` (open), `token` (static bearer/API key), or
   * `oauth` (browser sign-in). Registry metadata is unreliable, so this is the
   * source of truth.
   */
  detectAuth: async (
    server_id: string
  ): Promise<{
    kind: 'none' | 'token' | 'oauth';
    authorization_endpoint?: string;
    grant_types: string[];
  }> => {
    log('detect_auth server_id=%s', server_id);
    const result = await callCoreRpc<{
      kind: 'none' | 'token' | 'oauth';
      authorization_endpoint?: string;
      grant_types: string[];
    }>({ method: 'openhuman.mcp_clients_detect_auth', params: { server_id } });
    log('detect_auth -> %s', result.kind);
    return result;
  },

  /**
   * Begin browser OAuth (discover + dynamic client registration + PKCE) and
   * return the live authorize URL to open in a browser. The core's
   * `/oauth/mcp/callback` route completes the exchange and reconnects.
   */
  oauthBegin: async (server_id: string): Promise<string> => {
    log('oauth_begin server_id=%s', server_id);
    const result = await callCoreRpc<{ authorize_url: string }>({
      method: 'openhuman.mcp_clients_oauth_begin',
      params: { server_id },
    });
    log('oauth_begin returned authorize_url');
    return result.authorize_url;
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

  /**
   * Replace the stored env values for an installed server and reconnect so the
   * new credentials take effect (reconfigure / rotate keys without
   * uninstall+reinstall). `status` is `connected` when the reconnect succeeded.
   */
  updateEnv: async (params: {
    server_id: string;
    env: Record<string, string>;
  }): Promise<UpdateEnvResult> => {
    log('update_env server_id=%s env_keys=%o', params.server_id, Object.keys(params.env));
    const result = await callCoreRpc<UpdateEnvResult>({
      method: 'openhuman.mcp_clients_update_env',
      params,
    });
    log('update_env status=%s', result.status);
    return result;
  },

  /** Read which registry credentials are configured (booleans only, no values). */
  registrySettingsGet: async (): Promise<RegistrySettings> => {
    log('registry_settings_get');
    const result = await callCoreRpc<RegistrySettings>({
      method: 'openhuman.mcp_clients_registry_settings_get',
      params: {},
    });
    log(
      'registry_settings_get smithery=%s official=%s',
      result.smithery_api_key_set,
      result.mcp_official_token_set
    );
    return result;
  },

  /**
   * Persist registry credentials. Omit a field to leave it unchanged, pass an
   * empty string to clear it. Secrets are write-only — the response is the same
   * non-secret snapshot as registrySettingsGet.
   */
  registrySettingsSet: async (params: {
    smithery_api_key?: string;
    mcp_official_base?: string;
    mcp_official_token?: string;
  }): Promise<RegistrySettings> => {
    log('registry_settings_set fields=%o', Object.keys(params));
    const result = await callCoreRpc<RegistrySettings>({
      method: 'openhuman.mcp_clients_registry_settings_set',
      params,
    });
    return result;
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

  /** Enable or disable a server. Returns the new enabled state. */
  setEnabled: async (server_id: string, enabled: boolean): Promise<SetEnabledResult> => {
    log('set_enabled server_id=%s enabled=%s', server_id, enabled);
    const result = await callCoreRpc<SetEnabledResult>({
      method: 'openhuman.mcp_clients_set_enabled',
      params: { server_id, enabled },
    });
    log('set_enabled server_id=%s enabled=%s', result.server_id, result.enabled);
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
      // config_assist now runs a full agent turn (web search + fetch to read
      // the provider's docs), which legitimately takes far longer than the 30s
      // default RPC budget. Give it a generous 5-minute ceiling.
      timeoutMs: 300_000,
    });
    log(
      'config_assist reply length=%d suggested_env=%s',
      result.reply?.length ?? 0,
      result.suggested_env ? 'yes' : 'no'
    );
    return result;
  },
};
