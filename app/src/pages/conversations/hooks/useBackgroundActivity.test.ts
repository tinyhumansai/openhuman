import { act, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../../../services/coreRpcClient';
import { isTauri } from '../../../utils/tauriCommands/common';
import { openhumanCronList } from '../../../utils/tauriCommands/cron';
import { memorySyncStatusList } from '../../../utils/tauriCommands/memoryTree';
import {
  subconsciousStatus,
  subconsciousTriggersStatus,
} from '../../../utils/tauriCommands/subconscious';
import { useBackgroundActivity, useMemorySyncActive } from './useBackgroundActivity';

vi.mock('../../../utils/tauriCommands/common', () => ({ isTauri: vi.fn(() => true) }));
vi.mock('../../../utils/tauriCommands/cron', () => ({ openhumanCronList: vi.fn() }));
vi.mock('../../../utils/tauriCommands/memoryTree', () => ({ memorySyncStatusList: vi.fn() }));
vi.mock('../../../utils/tauriCommands/subconscious', () => ({
  subconsciousStatus: vi.fn(),
  subconsciousTriggersStatus: vi.fn(),
}));
vi.mock('../../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const mockCron = vi.mocked(openhumanCronList);
const mockSyncList = vi.mocked(memorySyncStatusList);
const mockSubStatus = vi.mocked(subconsciousStatus);
const mockTriggers = vi.mocked(subconsciousTriggersStatus);
const mockRpc = vi.mocked(callCoreRpc);
const mockIsTauri = vi.mocked(isTauri);

function fixtures() {
  mockCron.mockResolvedValue({
    result: [
      {
        id: 'j1',
        expression: '',
        schedule: { kind: 'cron', expr: '0 9 * * *' },
        command: '',
        job_type: 'agent',
        session_target: 'isolated',
        enabled: true,
        delivery: { mode: 'silent', best_effort: true },
        delete_after_run: false,
        created_at: '2024-01-01T00:00:00Z',
        next_run: '2999-01-01T00:00:00Z',
      },
    ],
    logs: [],
  });
  mockSubStatus.mockResolvedValue({
    result: {
      enabled: true,
      mode: 'event_driven',
      provider_available: true,
      provider_unavailable_reason: null,
      interval_minutes: 5,
      last_tick_at: 1_700_000_000,
      total_ticks: 12,
      consecutive_failures: 0,
    },
    logs: [],
  });
  mockTriggers.mockResolvedValue({
    result: {
      triggers_enabled: true,
      mode: 'event_driven',
      max_promotions_per_hour: 10,
      orchestrator_running: true,
      queue_depth: 2,
      orchestrator_thread_id: 'subconscious:orchestrator',
      user_thread_id: 'subconscious:user',
    },
    logs: [],
  });
  mockRpc.mockResolvedValue({ running: true, current_title: 'Inbox', queue_depth: 1 });
  mockSyncList.mockResolvedValue([
    {
      provider: 'slack',
      chunks_synced: 5,
      chunks_pending: 0,
      batch_total: 0,
      batch_processed: 0,
      last_chunk_at_ms: null,
      freshness: 'active',
    },
  ]);
}

describe('useBackgroundActivity', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockIsTauri.mockReturnValue(true);
    fixtures();
  });
  afterEach(() => vi.restoreAllMocks());

  it('does not fetch while closed', () => {
    renderHook(() => useBackgroundActivity(false));
    expect(mockCron).not.toHaveBeenCalled();
  });

  it('loads cron, subconscious and memory once open', async () => {
    const { result } = renderHook(() => useBackgroundActivity(true));
    await waitFor(() => expect(result.current.cronJobs).toHaveLength(1));
    expect(result.current.subconscious).toMatchObject({
      mode: 'event_driven',
      totalTicks: 12,
      working: true,
      queueDepth: 2,
    });
    expect(result.current.memory).toMatchObject({
      ingesting: true,
      currentTitle: 'Inbox',
      queueDepth: 1,
    });
    expect(result.current.memory.providers).toHaveLength(1);
  });

  it('surfaces nothing (and stops loading) outside Tauri', async () => {
    mockIsTauri.mockReturnValue(false);
    const { result } = renderHook(() => useBackgroundActivity(true));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(mockCron).not.toHaveBeenCalled();
    expect(result.current.cronJobs).toHaveLength(0);
  });

  it('tolerates a failing source without dropping the others', async () => {
    mockCron.mockRejectedValue(new Error('boom'));
    const { result } = renderHook(() => useBackgroundActivity(true));
    await waitFor(() => expect(result.current.subconscious).not.toBeNull());
    expect(result.current.cronJobs).toHaveLength(0);
    expect(result.current.memory.ingesting).toBe(true);
  });
});

describe('useMemorySyncActive', () => {
  function dispatch(detail: Record<string, unknown>) {
    act(() => {
      window.dispatchEvent(new CustomEvent('openhuman:memory-sync-stage', { detail }));
    });
  }

  it('lights up on a non-terminal stage and clears on a terminal one', () => {
    const { result } = renderHook(() => useMemorySyncActive());
    expect(result.current).toBe(false);

    dispatch({ stage: 'ingesting', source_id: 'src-1' });
    expect(result.current).toBe(true);

    dispatch({ stage: 'completed', source_id: 'src-1' });
    expect(result.current).toBe(false);
  });

  it('stays active until every source settles', () => {
    const { result } = renderHook(() => useMemorySyncActive());
    dispatch({ stage: 'fetching', source_id: 'a' });
    dispatch({ stage: 'fetching', source_id: 'b' });
    expect(result.current).toBe(true);

    dispatch({ stage: 'failed', source_id: 'a' });
    expect(result.current).toBe(true); // b still syncing

    dispatch({ stage: 'completed', source_id: 'b' });
    expect(result.current).toBe(false);
  });
});
