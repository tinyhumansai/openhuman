import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { useConsciousItems } from './useConsciousItems';

const mockConsciousLoopRun = vi.fn();
const mockMemoryQueryNamespace = vi.fn();

vi.mock('../utils/tauriCommands', async importOriginal => {
  const actual = await importOriginal<typeof import('../utils/tauriCommands')>();
  return {
    ...actual,
    isTauri: () => true,
    consciousLoopRun: (...args: unknown[]) => mockConsciousLoopRun(...args),
    memoryQueryNamespace: (...args: unknown[]) => mockMemoryQueryNamespace(...args),
  };
});

// `listen` resolves an unlisten fn but never fires events, so `isRunning` stays
// false — modelling the window before the backend `conscious_loop:started`
// event arrives, which is exactly where the duplicate-run race lived.
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));

vi.mock('../lib/coreState/store', () => ({
  getCoreStateSnapshot: () => ({ snapshot: { sessionToken: 'test-token' } }),
}));

vi.mock('../services/backendUrl', () => ({
  getBackendUrl: () => Promise.resolve('http://localhost:9999'),
}));

describe('useConsciousItems · triggerAnalysis re-entry guard', () => {
  beforeEach(() => {
    mockConsciousLoopRun.mockReset();
    mockMemoryQueryNamespace.mockReset();
    mockMemoryQueryNamespace.mockResolvedValue({ text: '' });
  });

  it('does not start a second run while the first is in flight (before isRunning flips)', async () => {
    let resolveRun: () => void = () => {};
    mockConsciousLoopRun.mockImplementation(
      () =>
        new Promise<void>(resolve => {
          resolveRun = resolve;
        })
    );

    const { result } = renderHook(() => useConsciousItems());

    // Two synchronous triggers within the pre-`started`-event window: the
    // backend has not yet flipped `isRunning`, so only the synchronous ref
    // guard can stop the duplicate.
    await act(async () => {
      void result.current.triggerAnalysis();
      void result.current.triggerAnalysis();
    });

    expect(mockConsciousLoopRun).toHaveBeenCalledTimes(1);

    // Settle the in-flight run so the ref releases for the next assertion.
    await act(async () => {
      resolveRun();
    });
  });

  it('releases the guard after a run settles so a later run can start', async () => {
    mockConsciousLoopRun.mockResolvedValue(undefined);

    const { result } = renderHook(() => useConsciousItems());

    await act(async () => {
      await result.current.triggerAnalysis();
    });
    await act(async () => {
      await result.current.triggerAnalysis();
    });

    expect(mockConsciousLoopRun).toHaveBeenCalledTimes(2);
  });
});
