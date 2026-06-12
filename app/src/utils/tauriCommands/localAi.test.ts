import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

describe('openhumanLocalAiTestConnection', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('calls callCoreRpc with inference_test_connection method and url param', async () => {
    const { callCoreRpc } = await import('../../services/coreRpcClient');
    const mockCallCoreRpc = callCoreRpc as ReturnType<typeof vi.fn>;
    mockCallCoreRpc.mockResolvedValueOnce({ reachable: true, models_count: 4 });

    const { openhumanLocalAiTestConnection } = await import('./localAi');
    const result = await openhumanLocalAiTestConnection('http://localhost:11434');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.inference_test_connection',
      params: { url: 'http://localhost:11434' },
    });
    expect(result).toEqual({ reachable: true, models_count: 4 });
  });

  it('propagates errors from callCoreRpc', async () => {
    const { callCoreRpc } = await import('../../services/coreRpcClient');
    const mockCallCoreRpc = callCoreRpc as ReturnType<typeof vi.fn>;
    mockCallCoreRpc.mockRejectedValueOnce(new Error('rpc down'));

    const { openhumanLocalAiTestConnection } = await import('./localAi');
    await expect(openhumanLocalAiTestConnection('http://localhost:11434')).rejects.toThrow(
      'rpc down'
    );
  });
});
