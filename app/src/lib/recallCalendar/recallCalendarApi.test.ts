import { beforeEach, describe, expect, it, vi } from 'vitest';

import { connect, disconnect, status } from './recallCalendarApi';

const callCoreRpc = vi.fn();
vi.mock('../../services/coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => callCoreRpc(...args),
}));

beforeEach(() => callCoreRpc.mockReset());

describe('recallCalendarApi', () => {
  it('connect returns connectUrl, unwrapping the CLI envelope', async () => {
    callCoreRpc.mockResolvedValue({ result: { connectUrl: 'https://consent' }, logs: [] });
    await expect(connect()).resolves.toEqual({ connectUrl: 'https://consent' });
    expect(callCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.recall_calendar_connect' });
  });

  it('status passes through an already-flat response', async () => {
    callCoreRpc.mockResolvedValue({ enabled: true, connected: true, email: 'a@b.com' });
    await expect(status()).resolves.toEqual({ enabled: true, connected: true, email: 'a@b.com' });
    expect(callCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.recall_calendar_status' });
  });

  it('disconnect unwraps an envelope carrying logs', async () => {
    callCoreRpc.mockResolvedValue({
      result: { disconnected: true },
      logs: ['calendar disconnected'],
    });
    await expect(disconnect()).resolves.toEqual({ disconnected: true });
    expect(callCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.recall_calendar_disconnect' });
  });
});
