import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { useSubconscious } from '../useSubconscious';

const mockStatus = {
  result: {
    instance: 'memory',
    enabled: true,
    mode: 'simple',
    provider_available: true,
    provider_unavailable_reason: null,
    interval_minutes: 30,
    last_tick_at: null,
    total_ticks: 0,
    consecutive_failures: 0,
  },
  logs: [],
};

const mockSettings = {
  result: {
    settings: {
      enabled: true,
      interval_minutes: 30,
      inference_enabled: true,
      notify_meetings: false,
      notify_reminders: false,
      notify_relevant_events: false,
      external_delivery_enabled: false,
      meeting_lookahead_minutes: 120,
      max_calendar_connections_per_tick: 2,
      reminder_lookahead_minutes: 30,
      subconscious_mode: 'simple' as const,
    },
  },
  logs: [],
};

let currentMode = 'simple';

vi.mock('../../utils/tauriCommands', () => ({
  isTauri: () => true,
  subconsciousStatus: vi.fn(async () => mockStatus),
  subconsciousTrigger: vi.fn(async () => ({ result: { triggered: true }, logs: [] })),
  openhumanHeartbeatSettingsGet: vi.fn(async () => ({
    result: { settings: { ...mockSettings.result.settings, subconscious_mode: currentMode } },
    logs: [],
  })),
  openhumanHeartbeatSettingsSet: vi.fn(async (patch: Record<string, unknown>) => {
    if (patch.subconscious_mode) currentMode = patch.subconscious_mode as string;
    return {
      result: {
        settings: { ...mockSettings.result.settings, ...patch, subconscious_mode: currentMode },
      },
      logs: [],
    };
  }),
}));

describe('useSubconscious', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
    currentMode = 'simple';
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('loads status and mode on mount', async () => {
    const { result } = renderHook(() => useSubconscious());

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(result.current.mode).toBe('simple');
    expect(result.current.intervalMinutes).toBe(30);
    expect(result.current.status).not.toBeNull();
  });

  it('setMode calls heartbeat settings set', async () => {
    const { openhumanHeartbeatSettingsSet } = await import('../../utils/tauriCommands');
    const { result } = renderHook(() => useSubconscious());

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    await act(async () => {
      await result.current.setMode('aggressive');
    });

    expect(openhumanHeartbeatSettingsSet).toHaveBeenCalledWith({ subconscious_mode: 'aggressive' });
    expect(result.current.mode).toBe('aggressive');
  });

  it('setIntervalMinutes calls heartbeat settings set', async () => {
    const { openhumanHeartbeatSettingsSet } = await import('../../utils/tauriCommands');
    const { result } = renderHook(() => useSubconscious());

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    await act(async () => {
      await result.current.setIntervalMinutes(15);
    });

    expect(openhumanHeartbeatSettingsSet).toHaveBeenCalledWith({ interval_minutes: 15 });
  });

  it('triggerTick calls subconsciousTrigger', async () => {
    const { subconsciousTrigger } = await import('../../utils/tauriCommands');
    const { result } = renderHook(() => useSubconscious());

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    await act(async () => {
      await result.current.triggerTick();
    });

    expect(subconsciousTrigger).toHaveBeenCalled();
  });

  it('triggerTick passes the kind through (no-arg = memory)', async () => {
    const { subconsciousTrigger } = await import('../../utils/tauriCommands');
    const { result } = renderHook(() => useSubconscious());

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    await act(async () => {
      await result.current.triggerTick('all');
    });
    expect(subconsciousTrigger).toHaveBeenLastCalledWith('all');

    await act(async () => {
      await result.current.triggerTick();
    });
    // A no-arg call maps to the legacy memory-only trigger.
    expect(subconsciousTrigger).toHaveBeenLastCalledWith(undefined);
  });

  it('derives a memory instance row when the core omits instances[]', async () => {
    const { result } = renderHook(() => useSubconscious());

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(result.current.instances).toHaveLength(1);
    expect(result.current.instances[0].instance).toBe('memory');
    // No kind is in flight initially.
    expect(result.current.isTriggering('memory')).toBe(false);
    expect(result.current.isTriggering('all')).toBe(false);
  });
});
