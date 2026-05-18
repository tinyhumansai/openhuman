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
          max_calendar_connections_per_tick: 2,
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

  test('runs a planner tick now', async () => {
    const { openhumanHeartbeatTickNow } = await import('./heartbeat');
    mockCallCoreRpc.mockResolvedValue({
      result: {
        summary: {
          source_events: 3,
          deliveries_attempted: 2,
          deliveries_sent: 1,
          deliveries_skipped_dedup: 1,
        },
      },
      logs: [],
    });

    const out = await openhumanHeartbeatTickNow();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.heartbeat_tick_now' });
    expect(out.result.summary.deliveries_sent).toBe(1);
  });

  test('rejects when not running in Tauri', async () => {
    mockIsTauri.mockReturnValue(false);
    const {
      openhumanHeartbeatSettingsGet,
      openhumanHeartbeatSettingsSet,
      openhumanHeartbeatTickNow,
    } = await import('./heartbeat');

    await expect(openhumanHeartbeatSettingsGet()).rejects.toThrow('Not running in Tauri');
    await expect(openhumanHeartbeatSettingsSet({ enabled: true })).rejects.toThrow(
      'Not running in Tauri'
    );
    await expect(openhumanHeartbeatTickNow()).rejects.toThrow('Not running in Tauri');
    expect(mockCallCoreRpc).not.toHaveBeenCalled();
  });
});
