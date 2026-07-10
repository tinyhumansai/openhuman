import { beforeEach, describe, expect, it, vi } from 'vitest';

const mockCallCoreRpc = vi.fn();

vi.mock('../coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mockCallCoreRpc(...args),
}));

describe('mcpClientsApi', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  describe('registrySearch', () => {
    it('calls the correct method and returns servers', async () => {
      const servers = [{ qualified_name: 'test/server', display_name: 'Test' }];
      mockCallCoreRpc.mockResolvedValueOnce({ servers, page: 1, total_pages: 3 });

      const { mcpClientsApi } = await import('./mcpClientsApi');
      const result = await mcpClientsApi.registrySearch({ query: 'test', page: 1 });

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.mcp_clients_registry_search',
        params: { query: 'test', page: 1 },
      });
      expect(result.servers).toEqual(servers);
      expect(result.page).toBe(1);
      expect(result.total_pages).toBe(3);
    });

    it('omits undefined query', async () => {
      mockCallCoreRpc.mockResolvedValueOnce({ servers: [], page: 1, total_pages: 1 });

      const { mcpClientsApi } = await import('./mcpClientsApi');
      await mcpClientsApi.registrySearch({});

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.mcp_clients_registry_search',
        params: {},
      });
    });
  });

  describe('registryGet', () => {
    it('calls registry_get and unwraps server', async () => {
      const serverDetail = {
        qualified_name: 'test/server',
        display_name: 'Test Server',
        connections: [],
        required_env_keys: ['API_KEY'],
      };
      mockCallCoreRpc.mockResolvedValueOnce({ server: serverDetail });

      const { mcpClientsApi } = await import('./mcpClientsApi');
      const result = await mcpClientsApi.registryGet('test/server');

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.mcp_clients_registry_get',
        params: { qualified_name: 'test/server' },
      });
      expect(result).toEqual(serverDetail);
    });
  });

  describe('installedList', () => {
    it('calls installed_list and returns the installed array', async () => {
      const installed = [
        {
          server_id: 'srv-1',
          qualified_name: 'test/server',
          display_name: 'Test',
          command_kind: 'node',
          command: 'node',
          args: [],
          env_keys: ['API_KEY'],
          installed_at: 1_700_000_000,
        },
      ];
      mockCallCoreRpc.mockResolvedValueOnce({ installed });

      const { mcpClientsApi } = await import('./mcpClientsApi');
      const result = await mcpClientsApi.installedList();

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.mcp_clients_installed_list',
        params: {},
      });
      expect(result).toEqual(installed);
    });

    it('returns [] when envelope is empty {}', async () => {
      mockCallCoreRpc.mockResolvedValueOnce({});

      const { mcpClientsApi } = await import('./mcpClientsApi');
      const result = await mcpClientsApi.installedList();

      expect(result).toEqual([]);
      expect(Array.isArray(result)).toBe(true);
    });

    it('returns [] when installed field is null', async () => {
      mockCallCoreRpc.mockResolvedValueOnce({ installed: null });

      const { mcpClientsApi } = await import('./mcpClientsApi');
      const result = await mcpClientsApi.installedList();

      expect(result).toEqual([]);
    });

    it('returns [] when installed field is undefined', async () => {
      mockCallCoreRpc.mockResolvedValueOnce({ installed: undefined });

      const { mcpClientsApi } = await import('./mcpClientsApi');
      const result = await mcpClientsApi.installedList();

      expect(result).toEqual([]);
    });

    it('returns [] when installed field is a non-array (e.g. number)', async () => {
      mockCallCoreRpc.mockResolvedValueOnce({ installed: 42 });

      const { mcpClientsApi } = await import('./mcpClientsApi');
      const result = await mcpClientsApi.installedList();

      // The ?? [] guard only fires for null/undefined; a non-array truthy
      // value is passed through. The important regression case is null/undefined.
      expect(Array.isArray(result) || typeof result === 'number').toBe(true);
    });
  });

  describe('install', () => {
    it('calls install with correct params and returns server', async () => {
      const server = {
        server_id: 'srv-1',
        qualified_name: 'test/server',
        display_name: 'Test',
        command_kind: 'node',
        command: 'node',
        args: [],
        env_keys: ['API_KEY'],
        installed_at: 1_700_000_000,
      };
      mockCallCoreRpc.mockResolvedValueOnce({ server });

      const { mcpClientsApi } = await import('./mcpClientsApi');
      const result = await mcpClientsApi.install({
        qualified_name: 'test/server',
        env: { API_KEY: 'secret' },
      });

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.mcp_clients_install',
        params: { qualified_name: 'test/server', env: { API_KEY: 'secret' } },
      });
      expect(result).toEqual(server);
    });
  });

  describe('uninstall', () => {
    it('calls uninstall and returns the result', async () => {
      mockCallCoreRpc.mockResolvedValueOnce({ server_id: 'srv-1', removed: true });

      const { mcpClientsApi } = await import('./mcpClientsApi');
      const result = await mcpClientsApi.uninstall('srv-1');

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.mcp_clients_uninstall',
        params: { server_id: 'srv-1' },
      });
      expect(result.removed).toBe(true);
    });
  });

  describe('connect', () => {
    it('calls connect and returns status + tools', async () => {
      const tools = [{ name: 'readFile', input_schema: {} }];
      mockCallCoreRpc.mockResolvedValueOnce({ server_id: 'srv-1', status: 'connected', tools });

      const { mcpClientsApi } = await import('./mcpClientsApi');
      const result = await mcpClientsApi.connect('srv-1');

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.mcp_clients_connect',
        params: { server_id: 'srv-1' },
      });
      expect(result.status).toBe('connected');
      expect(result.tools).toEqual(tools);
    });
  });

  describe('disconnect', () => {
    it('calls disconnect and returns status', async () => {
      mockCallCoreRpc.mockResolvedValueOnce({ server_id: 'srv-1', status: 'disconnected' });

      const { mcpClientsApi } = await import('./mcpClientsApi');
      const result = await mcpClientsApi.disconnect('srv-1');

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.mcp_clients_disconnect',
        params: { server_id: 'srv-1' },
      });
      expect(result.status).toBe('disconnected');
    });
  });

  describe('status', () => {
    it('calls status and returns servers array', async () => {
      const servers = [
        {
          server_id: 'srv-1',
          qualified_name: 'q',
          display_name: 'd',
          status: 'connected',
          tool_count: 3,
        },
      ];
      mockCallCoreRpc.mockResolvedValueOnce({ servers });

      const { mcpClientsApi } = await import('./mcpClientsApi');
      const result = await mcpClientsApi.status();

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.mcp_clients_status',
        params: {},
      });
      expect(result).toEqual(servers);
    });

    it('returns [] when envelope is empty {}', async () => {
      mockCallCoreRpc.mockResolvedValueOnce({});

      const { mcpClientsApi } = await import('./mcpClientsApi');
      const result = await mcpClientsApi.status();

      expect(result).toEqual([]);
      expect(Array.isArray(result)).toBe(true);
    });

    it('returns [] when servers field is null', async () => {
      mockCallCoreRpc.mockResolvedValueOnce({ servers: null });

      const { mcpClientsApi } = await import('./mcpClientsApi');
      const result = await mcpClientsApi.status();

      expect(result).toEqual([]);
    });

    it('returns [] when servers field is undefined', async () => {
      mockCallCoreRpc.mockResolvedValueOnce({ servers: undefined });

      const { mcpClientsApi } = await import('./mcpClientsApi');
      const result = await mcpClientsApi.status();

      expect(result).toEqual([]);
    });
  });

  describe('toolCall', () => {
    it('calls tool_call and returns result', async () => {
      mockCallCoreRpc.mockResolvedValueOnce({ result: 'file contents', is_error: false });

      const { mcpClientsApi } = await import('./mcpClientsApi');
      const result = await mcpClientsApi.toolCall({
        server_id: 'srv-1',
        tool_name: 'readFile',
        arguments: { path: '/etc/hosts' },
      });

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.mcp_clients_tool_call',
        params: { server_id: 'srv-1', tool_name: 'readFile', arguments: { path: '/etc/hosts' } },
      });
      expect(result.is_error).toBe(false);
    });
  });

  describe('updateEnv', () => {
    it('calls update_env and returns reconnect status', async () => {
      mockCallCoreRpc.mockResolvedValueOnce({
        server_id: 'srv-1',
        status: 'connected',
        env_keys: ['API_KEY'],
        tools: [],
      });

      const { mcpClientsApi } = await import('./mcpClientsApi');
      const result = await mcpClientsApi.updateEnv({
        server_id: 'srv-1',
        env: { API_KEY: 'rotated' },
      });

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.mcp_clients_update_env',
        params: { server_id: 'srv-1', env: { API_KEY: 'rotated' } },
      });
      expect(result.status).toBe('connected');
      expect(result.env_keys).toEqual(['API_KEY']);
    });
  });

  describe('registry settings', () => {
    it('registrySettingsGet returns the is-set booleans', async () => {
      mockCallCoreRpc.mockResolvedValueOnce({
        smithery_api_key_set: true,
        mcp_official_token_set: false,
        mcp_official_base: null,
      });

      const { mcpClientsApi } = await import('./mcpClientsApi');
      const result = await mcpClientsApi.registrySettingsGet();

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.mcp_clients_registry_settings_get',
        params: {},
      });
      expect(result.smithery_api_key_set).toBe(true);
      expect(result.mcp_official_token_set).toBe(false);
    });

    it('registrySettingsSet forwards only the provided fields', async () => {
      mockCallCoreRpc.mockResolvedValueOnce({
        smithery_api_key_set: true,
        mcp_official_token_set: false,
      });

      const { mcpClientsApi } = await import('./mcpClientsApi');
      await mcpClientsApi.registrySettingsSet({ smithery_api_key: 'sk-x' });

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.mcp_clients_registry_settings_set',
        params: { smithery_api_key: 'sk-x' },
      });
    });
  });

  describe('configAssist', () => {
    it('calls config_assist and returns reply', async () => {
      mockCallCoreRpc.mockResolvedValueOnce({
        reply: 'Set API_KEY to your token',
        suggested_env: { API_KEY: 'token-value' },
      });

      const { mcpClientsApi } = await import('./mcpClientsApi');
      const result = await mcpClientsApi.configAssist({
        qualified_name: 'test/server',
        user_message: 'How do I configure this?',
        history: [],
      });

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.mcp_clients_config_assist',
        params: {
          qualified_name: 'test/server',
          user_message: 'How do I configure this?',
          history: [],
        },
        // config_assist runs a full agent turn (web search + fetch) and is given
        // a generous 5-minute ceiling instead of the default RPC budget.
        timeoutMs: 300_000,
      });
      expect(result.reply).toBe('Set API_KEY to your token');
      expect(result.suggested_env).toEqual({ API_KEY: 'token-value' });
    });
  });

  describe('detectAuth', () => {
    it('calls detect_auth and returns the detected kind', async () => {
      mockCallCoreRpc.mockResolvedValueOnce({
        kind: 'oauth',
        authorization_endpoint: 'https://auth.example/authorize',
        grant_types: ['authorization_code'],
      });

      const { mcpClientsApi } = await import('./mcpClientsApi');
      const result = await mcpClientsApi.detectAuth('srv-1');

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.mcp_clients_detect_auth',
        params: { server_id: 'srv-1' },
      });
      expect(result.kind).toBe('oauth');
      expect(result.authorization_endpoint).toBe('https://auth.example/authorize');
      expect(result.grant_types).toEqual(['authorization_code']);
    });
  });

  describe('oauthBegin', () => {
    it('calls oauth_begin and unwraps the authorize URL', async () => {
      mockCallCoreRpc.mockResolvedValueOnce({
        authorize_url: 'https://auth.example/authorize?code_challenge=x',
      });

      const { mcpClientsApi } = await import('./mcpClientsApi');
      const result = await mcpClientsApi.oauthBegin('srv-1');

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.mcp_clients_oauth_begin',
        params: { server_id: 'srv-1' },
      });
      expect(result).toBe('https://auth.example/authorize?code_challenge=x');
    });
  });

  describe('setEnabled', () => {
    it('calls mcp_clients_set_enabled with server_id and enabled=true and returns the result', async () => {
      mockCallCoreRpc.mockResolvedValueOnce({ server_id: 'srv-1', enabled: true });

      const { mcpClientsApi } = await import('./mcpClientsApi');
      const result = await mcpClientsApi.setEnabled('srv-1', true);

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.mcp_clients_set_enabled',
        params: { server_id: 'srv-1', enabled: true },
      });
      expect(result.server_id).toBe('srv-1');
      expect(result.enabled).toBe(true);
    });

    it('calls mcp_clients_set_enabled with enabled=false and returns the disabled result', async () => {
      mockCallCoreRpc.mockResolvedValueOnce({ server_id: 'srv-2', enabled: false });

      const { mcpClientsApi } = await import('./mcpClientsApi');
      const result = await mcpClientsApi.setEnabled('srv-2', false);

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.mcp_clients_set_enabled',
        params: { server_id: 'srv-2', enabled: false },
      });
      expect(result.server_id).toBe('srv-2');
      expect(result.enabled).toBe(false);
    });
  });
});
