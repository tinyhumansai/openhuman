import { beforeEach, describe, expect, it, vi } from 'vitest';

const mockCallCoreRpc = vi.fn();

vi.mock('./coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mockCallCoreRpc(...args),
}));

describe('coreCommandClient', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  it('returns result from a successful CoreCommandResponse', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: 42, logs: [] });

    const { callCoreCommand } = await import('./coreCommandClient');
    const out = await callCoreCommand<number>('openhuman.some_method');

    expect(out).toBe(42);
  });

  it('forwards method name to callCoreRpc', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: 'ok', logs: [] });

    const { callCoreCommand } = await import('./coreCommandClient');
    await callCoreCommand('openhuman.webhooks_list_tunnels');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.webhooks_list_tunnels',
      params: undefined,
    });
  });

  it('forwards params to callCoreRpc when provided', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: { id: 't1' }, logs: [] });

    const { callCoreCommand } = await import('./coreCommandClient');
    const params = { id: 'tunnel-99', name: 'my-hook' };
    await callCoreCommand('openhuman.webhooks_update_tunnel', params);

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.webhooks_update_tunnel',
      params,
    });
  });

  it('returns object result intact', async () => {
    const payload = { tunnelId: 'abc', active: true };
    mockCallCoreRpc.mockResolvedValueOnce({ result: payload, logs: ['created'] });

    const { callCoreCommand } = await import('./coreCommandClient');
    const out = await callCoreCommand<typeof payload>('openhuman.webhooks_create_tunnel', {
      name: 'test',
    });

    expect(out).toEqual(payload);
  });

  it('propagates rejection from callCoreRpc', async () => {
    mockCallCoreRpc.mockRejectedValueOnce(new Error('core offline'));

    const { callCoreCommand } = await import('./coreCommandClient');
    await expect(callCoreCommand('openhuman.config_get')).rejects.toThrow('core offline');
  });

  it('handles undefined params gracefully (does not throw)', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: null, logs: [] });

    const { callCoreCommand } = await import('./coreCommandClient');
    await expect(callCoreCommand('openhuman.app_state_snapshot')).resolves.toBeNull();
  });

  it('returns null result when core responds with null', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: null, logs: [] });

    const { callCoreCommand } = await import('./coreCommandClient');
    const out = await callCoreCommand('openhuman.some_nullable_method');
    expect(out).toBeNull();
  });

  it('returns array result when core responds with an array', async () => {
    const list = [{ id: '1' }, { id: '2' }];
    mockCallCoreRpc.mockResolvedValueOnce({ result: list, logs: [] });

    const { callCoreCommand } = await import('./coreCommandClient');
    const out = await callCoreCommand<typeof list>('openhuman.list_items');
    expect(out).toEqual(list);
    expect(out).toHaveLength(2);
  });
});
