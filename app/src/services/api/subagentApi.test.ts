import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../coreRpcClient';
import { subagentApi } from './subagentApi';

vi.mock('../coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const mockCall = vi.mocked(callCoreRpc);

describe('subagentApi.cancel', () => {
  beforeEach(() => {
    mockCall.mockReset();
  });

  it('calls openhuman.subagent_cancel with the trimmed taskId', async () => {
    mockCall.mockResolvedValue({ cancelled: true, taskId: 'sub-1' });
    const result = await subagentApi.cancel('  sub-1  ');
    expect(mockCall).toHaveBeenCalledWith({
      method: 'openhuman.subagent_cancel',
      params: { taskId: 'sub-1' },
    });
    expect(result).toEqual({ cancelled: true, taskId: 'sub-1' });
  });

  it('forwards a non-empty reason and omits a blank one', async () => {
    mockCall.mockResolvedValue({ cancelled: true, taskId: 'sub-1' });
    await subagentApi.cancel('sub-1', '  user changed their mind  ');
    expect(mockCall).toHaveBeenCalledWith({
      method: 'openhuman.subagent_cancel',
      params: { taskId: 'sub-1', reason: 'user changed their mind' },
    });

    mockCall.mockClear();
    await subagentApi.cancel('sub-1', '   ');
    expect(mockCall).toHaveBeenCalledWith({
      method: 'openhuman.subagent_cancel',
      params: { taskId: 'sub-1' },
    });
  });

  it('rejects an empty taskId without calling core', async () => {
    await expect(subagentApi.cancel('   ')).rejects.toThrow('taskId is required');
    expect(mockCall).not.toHaveBeenCalled();
  });

  it('passes through cancelled=false (already finished / unknown)', async () => {
    mockCall.mockResolvedValue({ cancelled: false, taskId: 'sub-x' });
    const result = await subagentApi.cancel('sub-x');
    expect(result.cancelled).toBe(false);
  });
});
