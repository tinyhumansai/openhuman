import { beforeEach, describe, expect, it, vi } from 'vitest';

import { getCanvasTrackerSettings, updateCanvasTrackerSettings } from './canvasTrackerApi';

const { callCoreRpc } = vi.hoisted(() => ({
  callCoreRpc: vi.fn(),
}));

vi.mock('../../services/coreRpcClient', () => ({
  callCoreRpc,
}));

describe('canvasTrackerApi', () => {
  beforeEach(() => {
    callCoreRpc.mockReset();
  });

  it('unwraps RpcOutcome envelopes from settings reads', async () => {
    callCoreRpc.mockResolvedValue({
      result: {
        enabled: true,
        host: 'https://mango-cmu.instructure.com',
        token_set: false,
        allowlisted_courses: [],
      },
      logs: ['ok'],
    });

    await expect(getCanvasTrackerSettings()).resolves.toMatchObject({
      enabled: true,
      token_set: false,
    });
    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.canvas_tracker_get_settings',
    });
  });

  it('sends token only to update_settings and never returns it', async () => {
    callCoreRpc.mockResolvedValue({
      result: {
        enabled: true,
        host: 'https://mango-cmu.instructure.com',
        token_set: true,
        allowlisted_courses: [],
      },
      logs: [],
    });

    const result = await updateCanvasTrackerSettings({
      settings: {
        enabled: true,
        host: 'https://mango-cmu.instructure.com',
        token_set: false,
        allowlisted_courses: [],
      },
      token: 'canvas-secret-token',
    });

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.canvas_tracker_update_settings',
      params: {
        settings: {
          enabled: true,
          host: 'https://mango-cmu.instructure.com',
          token_set: false,
          allowlisted_courses: [],
        },
        token: 'canvas-secret-token',
      },
    });
    expect(result.token_set).toBe(true);
    expect(JSON.stringify(result)).not.toContain('canvas-secret-token');
  });
});
