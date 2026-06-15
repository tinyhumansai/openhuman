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

describe('agentWorkApi.control', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  function controlled(status: string): {
    row: AgentWorkResponse['groups'][number]['rows'][number];
  } {
    return {
      row: {
        runId: 'run-1',
        kind: 'subagent',
        bucket: status === 'cancelled' ? 'stopped' : 'working',
        status,
        startedAt: '2026-01-01T00:00:00Z',
        updatedAt: '2026-01-01T00:02:00Z',
        inputTokens: 0,
        outputTokens: 0,
        costUsd: 0,
        toolCount: 0,
      },
    };
  }

  it('stop forwards runId + action and returns the updated row', async () => {
    mockCall.mockResolvedValueOnce(controlled('cancelled'));
    const row = await agentWorkApi.control({ runId: 'run-1', action: 'stop' });
    expect(mockCall).toHaveBeenCalledWith({
      method: 'openhuman.agent_work_control',
      params: { runId: 'run-1', action: 'stop' },
    });
    expect(row.status).toBe('cancelled');
  });

  it('stop includes a trimmed reason when given', async () => {
    mockCall.mockResolvedValueOnce(controlled('cancelled'));
    await agentWorkApi.control({ runId: 'run-1', action: 'stop', reason: '  manual  ' });
    expect(mockCall).toHaveBeenCalledWith({
      method: 'openhuman.agent_work_control',
      params: { runId: 'run-1', action: 'stop', reason: 'manual' },
    });
  });

  it('continue forwards the trimmed message', async () => {
    mockCall.mockResolvedValueOnce(controlled('running'));
    await agentWorkApi.control({ runId: 'run-1', action: 'continue', message: '  go  ' });
    expect(mockCall).toHaveBeenCalledWith({
      method: 'openhuman.agent_work_control',
      params: { runId: 'run-1', action: 'continue', message: 'go' },
    });
  });

  it('rejects continue / follow_up without a message before calling core', async () => {
    await expect(agentWorkApi.control({ runId: 'run-1', action: 'continue' })).rejects.toThrow(
      'continue requires a message'
    );
    await expect(
      agentWorkApi.control({ runId: 'run-1', action: 'follow_up', message: '   ' })
    ).rejects.toThrow('follow_up requires a message');
    expect(mockCall).not.toHaveBeenCalled();
  });

  it('rejects a missing runId before calling core', async () => {
    await expect(agentWorkApi.control({ runId: '  ', action: 'retry' })).rejects.toThrow(
      'runId is required'
    );
    expect(mockCall).not.toHaveBeenCalled();
  });
});
