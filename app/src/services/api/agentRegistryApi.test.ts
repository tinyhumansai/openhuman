import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../coreRpcClient';
import { agentRegistryApi, type AgentRegistryEntry } from './agentRegistryApi';

vi.mock('../coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const mockCall = vi.mocked(callCoreRpc);

function agent(overrides: Partial<AgentRegistryEntry> = {}): AgentRegistryEntry {
  return {
    id: 'researcher',
    name: 'Researcher',
    description: 'Looks things up.',
    source: 'default',
    enabled: true,
    tool_allowlist: ['*'],
    ...overrides,
  };
}

describe('agentRegistryApi', () => {
  beforeEach(() => vi.clearAllMocks());

  it('list passes include_disabled and returns the agents array', async () => {
    mockCall.mockResolvedValueOnce({ agents: [agent()] });
    const res = await agentRegistryApi.list(true);
    expect(mockCall).toHaveBeenCalledWith({
      method: 'openhuman.agent_registry_list',
      params: { include_disabled: true },
    });
    expect(res).toHaveLength(1);
    expect(res[0].id).toBe('researcher');
  });

  it('list tolerates a missing agents field', async () => {
    mockCall.mockResolvedValueOnce({});
    expect(await agentRegistryApi.list()).toEqual([]);
  });

  it('createCustom prunes undefined fields', async () => {
    mockCall.mockResolvedValueOnce({ agent: agent({ id: 'finance', source: 'custom' }) });
    await agentRegistryApi.createCustom({
      id: 'finance',
      name: 'Finance',
      description: 'Money stuff.',
      model: null,
      tool_allowlist: ['memory.search'],
    });
    expect(mockCall).toHaveBeenCalledWith({
      method: 'openhuman.agent_registry_create_custom',
      params: {
        id: 'finance',
        name: 'Finance',
        description: 'Money stuff.',
        model: null,
        tool_allowlist: ['memory.search'],
      },
    });
  });

  it('update forwards id plus the patch', async () => {
    mockCall.mockResolvedValueOnce({ agent: agent({ name: 'Renamed' }) });
    const res = await agentRegistryApi.update('researcher', { name: 'Renamed' });
    expect(mockCall).toHaveBeenCalledWith({
      method: 'openhuman.agent_registry_update',
      params: { id: 'researcher', name: 'Renamed' },
    });
    expect(res.name).toBe('Renamed');
  });

  it('setEnabled sends id + enabled', async () => {
    mockCall.mockResolvedValueOnce({ agent: agent({ enabled: false }) });
    const res = await agentRegistryApi.setEnabled('researcher', false);
    expect(mockCall).toHaveBeenLastCalledWith({
      method: 'openhuman.agent_registry_set_enabled',
      params: { id: 'researcher', enabled: false },
    });
    expect(res.enabled).toBe(false);
  });

  it('remove returns the boolean outcome', async () => {
    mockCall.mockResolvedValueOnce({ removed: true });
    expect(await agentRegistryApi.remove('finance')).toBe(true);
    expect(mockCall).toHaveBeenLastCalledWith({
      method: 'openhuman.agent_registry_remove',
      params: { id: 'finance' },
    });
  });
  describe('availableTools', () => {
    it('calls agent_registry_available_tools and returns the tools', async () => {
      mockCall.mockResolvedValueOnce({
        tools: [
          { name: 'web_search', description: 'Search the web.' },
          { name: 'file_read', description: 'Read a file.' },
        ],
      });
      const res = await agentRegistryApi.availableTools();
      expect(mockCall).toHaveBeenCalledWith({
        method: 'openhuman.agent_registry_available_tools',
        params: {},
      });
      expect(res).toEqual([
        { name: 'web_search', description: 'Search the web.' },
        { name: 'file_read', description: 'Read a file.' },
      ]);
    });

    it('peels a { result, logs } CLI envelope', async () => {
      mockCall.mockResolvedValueOnce({
        result: { tools: [{ name: 'web_search', description: 'Search.' }] },
        logs: ['listed'],
      });
      expect(await agentRegistryApi.availableTools()).toEqual([
        { name: 'web_search', description: 'Search.' },
      ]);
    });

    it.each([[null], [undefined], [{}], [{ tools: null }], [{ tools: 'nope' }], [[]]])(
      'returns [] for an empty or malformed payload (%j)',
      async payload => {
        mockCall.mockResolvedValueOnce(payload);
        expect(await agentRegistryApi.availableTools()).toEqual([]);
      }
    );

    it('drops entries without a usable name and defaults a missing description', async () => {
      mockCall.mockResolvedValueOnce({
        tools: [
          null,
          'web_search',
          { description: 'no name' },
          { name: '   ', description: 'blank name' },
          { name: 42 },
          { name: ' shell ', description: 7 },
        ],
      });
      expect(await agentRegistryApi.availableTools()).toEqual([{ name: 'shell', description: '' }]);
    });
  });
});
