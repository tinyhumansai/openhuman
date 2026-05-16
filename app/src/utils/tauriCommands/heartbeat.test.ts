import { isTauri } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, type Mock, test, vi } from 'vitest';

import { callCoreRpc } from '../../services/coreRpcClient';

vi.mock('@tauri-apps/api/core', () => ({ isTauri: vi.fn() }));
vi.mock('../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

describe('tauriCommands/heartbeat', () => {
  const mockIsTauri = isTauri as Mock;
  const mockCallCoreRpc = callCoreRpc as Mock;

  beforeEach(() => {
    vi.clearAllMocks();
    mockIsTauri.mockReturnValue(true);
    Object.defineProperty(window, '__TAURI_INTERNALS__', {
      configurable: true,
      value: { invoke: vi.fn() },
    });
  });

  test('reads heartbeat settings', async () => {
    const { openhumanHeartbeatSettingsGet } = await import('./heartbeat');
    mockCallCoreRpc.mockResolvedValue({
      result: {
        settings: {
          enabled: false,
          interval_minutes: 5,
          inference_enabled: false,
          notify_meetings: false,
          notify_reminders: false,
          notify_relevant_events: false,
          external_delivery_enabled: false,
          meeting_lookahead_minutes: 120,
          reminder_lookahead_minutes: 30,
        },
      },
      logs: [],
    });

    const out = await openhumanHeartbeatSettingsGet();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.heartbeat_settings_get' });
    expect(out.result.settings.enabled).toBe(false);
  });

  test('saves heartbeat settings patch', async () => {
    const { openhumanHeartbeatSettingsSet } = await import('./heartbeat');
    mockCallCoreRpc.mockResolvedValue({ result: { settings: { enabled: true } }, logs: [] });

    await openhumanHeartbeatSettingsSet({ enabled: true, interval_minutes: 15 });

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.heartbeat_settings_set',
      params: { enabled: true, interval_minutes: 15 },
    });
  });
});
