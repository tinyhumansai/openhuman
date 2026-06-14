import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../coreRpcClient';
import { agentWorkApi, type AgentWorkResponse } from './agentWorkApi';

vi.mock('../coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const mockCall = vi.mocked(callCoreRpc);

function response(): AgentWorkResponse {
  return {
    total: 2,
    groups: [
      { bucket: 'needs_input', count: 0, rows: [] },
      {
        bucket: 'working',
        count: 1,
        rows: [
          {
            runId: 'run-1',
            kind: 'subagent',
            agentId: 'agent-a',
            displayName: 'Researcher',
            bucket: 'working',
            status: 'running',
            workerThreadId: 'thread-w',
            startedAt: '2026-01-01T00:00:00Z',
            updatedAt: '2026-01-01T00:01:00Z',
            elapsedMs: 60000,
            inputTokens: 100,
            outputTokens: 50,
            costUsd: 0.01,
            toolCount: 2,
          },
        ],
      },
      { bucket: 'completed', count: 1, rows: [] },
      { bucket: 'failed', count: 0, rows: [] },
      { bucket: 'stopped', count: 0, rows: [] },
    ],
  };
}

describe('agentWorkApi', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('list calls the agent_work_list RPC with no params when limit is omitted', async () => {
    mockCall.mockResolvedValueOnce(response());
    await agentWorkApi.list();
    expect(mockCall).toHaveBeenCalledWith({ method: 'openhuman.agent_work_list', params: {} });
  });

  it('list forwards an explicit limit', async () => {
    mockCall.mockResolvedValueOnce(response());
    await agentWorkApi.list(25);
    expect(mockCall).toHaveBeenCalledWith({
      method: 'openhuman.agent_work_list',
      params: { limit: 25 },
    });
  });

  it('list rejects a non-positive or non-integer limit without calling core', async () => {
    await expect(agentWorkApi.list(0)).rejects.toThrow('positive integer');
    await expect(agentWorkApi.list(-5)).rejects.toThrow('positive integer');
    await expect(agentWorkApi.list(1.5)).rejects.toThrow('positive integer');
    expect(mockCall).not.toHaveBeenCalled();
  });

  it('list returns the grouped response unchanged (wire is already camelCase)', async () => {
    mockCall.mockResolvedValueOnce(response());
    const result = await agentWorkApi.list();
    expect(result.total).toBe(2);
    expect(result.groups).toHaveLength(5);
    expect(result.groups.map(g => g.bucket)).toEqual([
      'needs_input',
      'working',
      'completed',
      'failed',
      'stopped',
    ]);
    const working = result.groups.find(g => g.bucket === 'working');
    expect(working?.count).toBe(1);
    expect(working?.rows[0].displayName).toBe('Researcher');
    expect(working?.rows[0].workerThreadId).toBe('thread-w');
  });
});
