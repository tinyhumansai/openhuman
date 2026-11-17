import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { Thread } from '../../../types/thread';
import { PINNED_THREAD_LABEL } from './groupThreads';
import { useThreadPins } from './useThreadPins';

const updateLabels = vi.fn();
const dispatch = vi.fn();

vi.mock('../../../services/api/threadApi', () => ({
  threadApi: { updateLabels: (...args: unknown[]) => updateLabels(...args) },
}));
vi.mock('../../../store/hooks', () => ({ useAppDispatch: () => dispatch }));
vi.mock('../../../store/threadSlice', () => ({
  loadThreads: () => ({ type: 'thread/loadThreads' }),
}));

const THREAD: Thread = {
  id: 't1',
  title: 'One',
  chatId: null,
  isActive: true,
  messageCount: 1,
  lastMessageAt: '2026-10-06T12:00:00Z',
  createdAt: '2026-10-06T12:00:00Z',
  labels: ['general'],
};

describe('useThreadPins', () => {
  beforeEach(() => {
    updateLabels.mockReset();
    dispatch.mockReset();
  });

  it('writes the pinned label, reloads threads and drops the override', async () => {
    let resolveUpdate: () => void = () => {};
    updateLabels.mockReturnValue(new Promise<void>(resolve => (resolveUpdate = resolve)));
    dispatch.mockReturnValue({ unwrap: () => Promise.resolve() });
    const { result } = renderHook(() => useThreadPins());

    act(() => result.current.togglePin(THREAD, true));
    // Optimistic: pinned before the core answers.
    expect(result.current.isPinned(THREAD)).toBe(true);
    expect(updateLabels).toHaveBeenCalledWith('t1', ['general', PINNED_THREAD_LABEL]);

    await act(async () => resolveUpdate());
    await waitFor(() => expect(dispatch).toHaveBeenCalledWith({ type: 'thread/loadThreads' }));
    // Override cleared: the (unchanged, mocked) thread record decides again.
    await waitFor(() => expect(result.current.isPinned(THREAD)).toBe(false));
  });

  it('reverts when the label write fails', async () => {
    updateLabels.mockRejectedValue(new Error('boom'));
    const pinned = { ...THREAD, labels: [PINNED_THREAD_LABEL] };
    const { result } = renderHook(() => useThreadPins());

    act(() => result.current.togglePin(pinned, false));
    expect(result.current.isPinned(pinned)).toBe(false);
    expect(updateLabels).toHaveBeenCalledWith('t1', []);
    await waitFor(() => expect(result.current.isPinned(pinned)).toBe(true));
    expect(dispatch).not.toHaveBeenCalled();
  });
});
