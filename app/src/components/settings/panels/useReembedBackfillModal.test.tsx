import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { memoryTreeBackfillStatus } from '../../../utils/tauriCommands/memoryTree';
import { useReembedBackfillModal } from './useReembedBackfillModal';

vi.mock('../../../utils/tauriCommands/memoryTree', () => ({ memoryTreeBackfillStatus: vi.fn() }));

const status = vi.mocked(memoryTreeBackfillStatus);

beforeEach(() => {
  vi.clearAllMocks();
});
afterEach(() => {
  vi.useRealTimers();
});

describe('useReembedBackfillModal (#1574 §4b)', () => {
  it('does not check status or open the modal when save fails', async () => {
    const save = vi.fn().mockResolvedValue(false);
    const { result } = renderHook(() => useReembedBackfillModal(save));

    await act(async () => {
      await result.current.handleSave();
    });

    expect(save).toHaveBeenCalledTimes(1);
    expect(status).not.toHaveBeenCalled();
    expect(result.current.reembed).toEqual({ open: false, pending: 0 });
  });

  it('opens the modal when save succeeds and a backfill is in progress', async () => {
    const save = vi.fn().mockResolvedValue(true);
    status.mockResolvedValue({ in_progress: true, pending_jobs: 7 });
    const { result } = renderHook(() => useReembedBackfillModal(save));

    await act(async () => {
      await result.current.handleSave();
    });

    expect(result.current.reembed).toEqual({ open: true, pending: 7 });
  });

  it('stays closed when save succeeds but no backfill is in progress', async () => {
    const save = vi.fn().mockResolvedValue(true);
    status.mockResolvedValue({ in_progress: false, pending_jobs: 0 });
    const { result } = renderHook(() => useReembedBackfillModal(save));

    await act(async () => {
      await result.current.handleSave();
    });

    expect(result.current.reembed.open).toBe(false);
  });

  it('swallows a status-check error and stays closed', async () => {
    const save = vi.fn().mockResolvedValue(true);
    status.mockRejectedValue(new Error('rpc down'));
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const { result } = renderHook(() => useReembedBackfillModal(save));

    await act(async () => {
      await result.current.handleSave();
    });

    expect(result.current.reembed.open).toBe(false);
    expect(warn).toHaveBeenCalled();
    warn.mockRestore();
  });

  it('polls while open: updates pending, then auto-closes when the backfill drains', async () => {
    vi.useFakeTimers();
    const save = vi.fn().mockResolvedValue(true);
    status
      .mockResolvedValueOnce({ in_progress: true, pending_jobs: 9 }) // handleSave open
      .mockResolvedValueOnce({ in_progress: true, pending_jobs: 4 }) // poll #1
      .mockResolvedValueOnce({ in_progress: false, pending_jobs: 0 }); // poll #2 → close
    const { result } = renderHook(() => useReembedBackfillModal(save));

    await act(async () => {
      await result.current.handleSave();
    });
    expect(result.current.reembed).toEqual({ open: true, pending: 9 });

    await act(async () => {
      await vi.advanceTimersByTimeAsync(2000);
    });
    expect(result.current.reembed.pending).toBe(4);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(2000);
    });
    expect(result.current.reembed.open).toBe(false);
  });

  it('dismissReembed closes the modal (background backfill unaffected)', async () => {
    const save = vi.fn().mockResolvedValue(true);
    status.mockResolvedValue({ in_progress: true, pending_jobs: 2 });
    const { result } = renderHook(() => useReembedBackfillModal(save));

    await act(async () => {
      await result.current.handleSave();
    });
    expect(result.current.reembed.open).toBe(true);

    act(() => {
      result.current.dismissReembed();
    });
    expect(result.current.reembed).toEqual({ open: false, pending: 0 });
  });
});
