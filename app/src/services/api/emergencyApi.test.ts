import { beforeEach, describe, expect, it, vi } from 'vitest';

import { emergencyResume, emergencyStatus, emergencyStop } from './emergencyApi';

const call = vi.fn();
vi.mock('../coreRpcClient', () => ({ callCoreRpc: (arg: unknown) => call(arg) }));

beforeEach(() => call.mockReset());

describe('emergencyApi', () => {
  it('emergencyStop calls openhuman.emergency_stop with reason and unwraps envelope', async () => {
    call.mockResolvedValue({ result: { engaged: true, reason: 'user' }, logs: ['x'] });
    const r = await emergencyStop('user');
    expect(call).toHaveBeenCalledWith({
      method: 'openhuman.emergency_stop',
      params: { reason: 'user' },
    });
    expect(r.engaged).toBe(true);
    expect(r.reason).toBe('user');
  });
  it('emergencyStop with no reason sends empty params', async () => {
    call.mockResolvedValue({ result: { engaged: true }, logs: [] });
    await emergencyStop();
    expect(call).toHaveBeenCalledWith({ method: 'openhuman.emergency_stop', params: {} });
  });
  it('emergencyResume calls openhuman.emergency_resume', async () => {
    call.mockResolvedValue({ result: { engaged: false }, logs: ['x'] });
    const r = await emergencyResume();
    expect(call).toHaveBeenCalledWith({ method: 'openhuman.emergency_resume', params: {} });
    expect(r.engaged).toBe(false);
  });
  it('emergencyStatus reads bare value (no envelope)', async () => {
    call.mockResolvedValue({ engaged: false });
    const r = await emergencyStatus();
    expect(call).toHaveBeenCalledWith({ method: 'openhuman.emergency_status', params: {} });
    expect(r.engaged).toBe(false);
  });
});
