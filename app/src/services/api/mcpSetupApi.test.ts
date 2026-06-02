import { beforeEach, describe, expect, it, vi } from 'vitest';

const mockCallCoreRpc = vi.fn();

vi.mock('../coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mockCallCoreRpc(...args),
}));

describe('mcpSetupApi', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  it('search calls mcp_setup_search', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ servers: [], page: 1, total_pages: 1 });
    const { mcpSetupApi } = await import('./mcpSetupApi');
    await mcpSetupApi.search({ query: 'notion' });
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.mcp_setup_search',
      params: { query: 'notion' },
    });
  });

  it('get unwraps the server detail', async () => {
    const server = { qualified_name: 'q', display_name: 'd', connections: [] };
    mockCallCoreRpc.mockResolvedValueOnce({ server });
    const { mcpSetupApi } = await import('./mcpSetupApi');
    const result = await mcpSetupApi.get('q');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.mcp_setup_get',
      params: { qualified_name: 'q' },
    });
    expect(result).toEqual(server);
  });

  it('requestSecret returns an opaque ref', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ ref: 'secret://abc', key_name: 'NOTION_API_KEY' });
    const { mcpSetupApi } = await import('./mcpSetupApi');
    const result = await mcpSetupApi.requestSecret({
      key_name: 'NOTION_API_KEY',
      prompt: 'Paste your token',
    });
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.mcp_setup_request_secret',
      params: { key_name: 'NOTION_API_KEY', prompt: 'Paste your token' },
    });
    expect(result.ref).toBe('secret://abc');
  });

  it('submitSecret forwards the ref + value', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ ref: 'secret://abc', fulfilled: true });
    const { mcpSetupApi } = await import('./mcpSetupApi');
    const result = await mcpSetupApi.submitSecret({ ref_id: 'secret://abc', value: 'tok' });
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.mcp_setup_submit_secret',
      params: { ref_id: 'secret://abc', value: 'tok' },
    });
    expect(result.fulfilled).toBe(true);
  });

  it('testConnection returns ok + tools', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ ok: true, tools: [{ name: 'search' }] });
    const { mcpSetupApi } = await import('./mcpSetupApi');
    const result = await mcpSetupApi.testConnection({
      qualified_name: 'q',
      env_refs: { NOTION_API_KEY: 'secret://abc' },
    });
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.mcp_setup_test_connection',
      params: { qualified_name: 'q', env_refs: { NOTION_API_KEY: 'secret://abc' } },
    });
    expect(result.ok).toBe(true);
  });

  it('installAndConnect commits the install', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ server_id: 'srv-1', status: 'connected', tools: [] });
    const { mcpSetupApi } = await import('./mcpSetupApi');
    const result = await mcpSetupApi.installAndConnect({
      qualified_name: 'q',
      env_refs: { NOTION_API_KEY: 'secret://abc' },
    });
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.mcp_setup_install_and_connect',
      params: { qualified_name: 'q', env_refs: { NOTION_API_KEY: 'secret://abc' } },
    });
    expect(result.status).toBe('connected');
  });
});
