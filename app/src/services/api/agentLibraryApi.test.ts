import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../coreRpcClient';
import { agentLibraryApi } from './agentLibraryApi';

vi.mock('../coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const mockCall = vi.mocked(callCoreRpc);

describe('agentLibraryApi', () => {
  beforeEach(() => vi.clearAllMocks());

  it('lists safe agent definitions through the agent controller', async () => {
    mockCall.mockResolvedValueOnce({
      definitions: [
        {
          id: 'researcher',
          display_name: 'Researcher',
          when_to_use: 'Use for research.',
          tier: 'worker',
          model: { kind: 'hint', value: 'reasoning' },
          direct_tool_count: 1,
          direct_tool_names: ['web_search'],
          uses_wildcard_tools: false,
          subagent_ids: [],
          includes_profile: false,
          includes_memory_md: false,
          includes_memory_context: false,
          can_run_as_user_facing_worker: true,
          write_capable: false,
          source: 'builtin',
        },
      ],
    });

    await expect(agentLibraryApi.listDefinitions()).resolves.toMatchObject([
      { id: 'researcher', write_capable: false },
    ]);
    expect(mockCall).toHaveBeenCalledWith({
      method: 'openhuman.agent_list_definitions',
      params: {},
    });
  });

  it('tolerates a missing definitions field', async () => {
    mockCall.mockResolvedValueOnce({});
    await expect(agentLibraryApi.listDefinitions()).resolves.toEqual([]);
  });

  it('tolerates a non-array definitions field', async () => {
    mockCall.mockResolvedValueOnce({ definitions: { id: 'bad-shape' } });
    await expect(agentLibraryApi.listDefinitions()).resolves.toEqual([]);
  });
});
