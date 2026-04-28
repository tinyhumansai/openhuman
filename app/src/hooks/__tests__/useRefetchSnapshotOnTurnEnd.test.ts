import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { useCoreState } from '../../providers/CoreStateProvider';
import { userApi } from '../../services/api/userApi';
import { useRefetchSnapshotOnTurnEnd } from '../useRefetchSnapshotOnTurnEnd';

vi.mock('../../providers/CoreStateProvider', () => ({ useCoreState: vi.fn() }));

vi.mock('../../services/api/userApi', () => ({ userApi: { getMe: vi.fn() } }));

describe('useRefetchSnapshotOnTurnEnd', () => {
  const mockPatchSnapshot = vi.fn();

  beforeEach(() => {
    vi.useFakeTimers();
    vi.mocked(useCoreState).mockReturnValue({ patchSnapshot: mockPatchSnapshot } as any);
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.useRealTimers();
  });

  it('refetches and patches snapshot after 750ms', async () => {
    const mockUser1 = { _id: 'user1', firstName: 'Jules' };
    vi.mocked(userApi.getMe).mockResolvedValue(mockUser1 as any);

    const { result } = renderHook(() => useRefetchSnapshotOnTurnEnd());

    act(() => {
      result.current.refetch();
    });

    expect(userApi.getMe).not.toHaveBeenCalled();

    await act(async () => {
      vi.advanceTimersByTime(750);
    });

    expect(userApi.getMe).toHaveBeenCalledTimes(1);
    expect(mockPatchSnapshot).toHaveBeenCalledWith({ currentUser: mockUser1 });
  });

  it('three rapid finalize events → one getMe call', async () => {
    const mockUser1 = { _id: 'user1', firstName: 'Jules' };
    vi.mocked(userApi.getMe).mockResolvedValue(mockUser1 as any);

    const { result } = renderHook(() => useRefetchSnapshotOnTurnEnd());

    act(() => {
      result.current.refetch();
      vi.advanceTimersByTime(300);
      result.current.refetch();
      vi.advanceTimersByTime(300);
      result.current.refetch();
    });

    expect(userApi.getMe).not.toHaveBeenCalled();

    await act(async () => {
      vi.advanceTimersByTime(750);
    });

    expect(userApi.getMe).toHaveBeenCalledTimes(1);
  });

  it('two sequential payloads → second value lands in snapshot', async () => {
    const mockUser1 = { _id: 'user1', firstName: 'Jules', hasAccess: false };
    const mockUser2 = { _id: 'user1', firstName: 'Jules', hasAccess: true };

    vi.mocked(userApi.getMe)
      .mockResolvedValueOnce(mockUser1 as any)
      .mockResolvedValueOnce(mockUser2 as any);

    const { result } = renderHook(() => useRefetchSnapshotOnTurnEnd());

    // First refetch
    act(() => {
      result.current.refetch();
    });
    await act(async () => {
      vi.advanceTimersByTime(750);
    });
    expect(mockPatchSnapshot).toHaveBeenLastCalledWith({ currentUser: mockUser1 });

    // Second refetch
    act(() => {
      result.current.refetch();
    });
    await act(async () => {
      vi.advanceTimersByTime(750);
    });
    expect(mockPatchSnapshot).toHaveBeenLastCalledWith({ currentUser: mockUser2 });
  });

  it('clears the pending debounce timer on unmount so getMe never fires', async () => {
    vi.mocked(userApi.getMe).mockResolvedValue({ _id: 'user1' } as any);

    const { result, unmount } = renderHook(() => useRefetchSnapshotOnTurnEnd());

    act(() => {
      result.current.refetch();
    });

    unmount();

    await act(async () => {
      vi.advanceTimersByTime(750);
    });

    expect(userApi.getMe).not.toHaveBeenCalled();
  });
});
