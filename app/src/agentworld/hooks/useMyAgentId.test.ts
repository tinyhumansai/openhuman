import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { useMyAgentId } from './useMyAgentId';

const { debugLog, fetchWalletStatus, setState } = vi.hoisted(() => ({
  debugLog: vi.fn(),
  fetchWalletStatus: vi.fn(),
  setState: vi.fn(),
}));

vi.mock('debug', () => ({ default: vi.fn(() => debugLog) }));

vi.mock('../../services/walletApi', () => ({ fetchWalletStatus }));

vi.mock('react', async importOriginal => {
  const react = await importOriginal<typeof import('react')>();

  return {
    ...react,
    useState: ((initialState: unknown) => {
      const [state, updateState] = react.useState(initialState);

      return [
        state,
        (nextState: unknown) => {
          setState(nextState);
          updateState(nextState);
        },
      ];
    }) as typeof react.useState,
  };
});

function walletStatus(accounts: Array<{ chain: string; address: string }>) {
  return { accounts };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

describe('useMyAgentId', () => {
  beforeEach(() => {
    debugLog.mockReset();
    fetchWalletStatus.mockReset();
    setState.mockReset();
  });

  test('starts loading, fetches once, and resolves a Solana account', async () => {
    const request = deferred<ReturnType<typeof walletStatus>>();
    fetchWalletStatus.mockReturnValue(request.promise);

    const { result } = renderHook(() => useMyAgentId());

    expect(result.current).toEqual({ status: 'loading' });
    expect(fetchWalletStatus).toHaveBeenCalledTimes(1);

    act(() => request.resolve(walletStatus([{ chain: 'solana', address: 'SolanaAgent123' }])));

    await waitFor(() =>
      expect(result.current).toEqual({ status: 'ready', agentId: 'SolanaAgent123' })
    );
  });

  test('resolves disconnected when there is no Solana account', async () => {
    fetchWalletStatus.mockResolvedValue(walletStatus([]));

    const { result } = renderHook(() => useMyAgentId());

    await waitFor(() => expect(result.current).toEqual({ status: 'disconnected' }));
  });

  test('ignores non-Solana accounts', async () => {
    fetchWalletStatus.mockResolvedValue(
      walletStatus([
        { chain: 'evm', address: '0x123' },
        { chain: 'btc', address: 'bc1abc' },
      ])
    );

    const { result } = renderHook(() => useMyAgentId());

    await waitFor(() => expect(result.current).toEqual({ status: 'disconnected' }));
  });

  test('preserves Error rejections without logging their content', async () => {
    const error = new Error('private wallet failure detail');
    fetchWalletStatus.mockRejectedValue(error);

    const { result } = renderHook(() => useMyAgentId());

    await waitFor(() => expect(result.current).toEqual({ status: 'error', error }));
    expect(debugLog).toHaveBeenCalled();
    expect(debugLog.mock.calls.flat().join(' ')).not.toContain(error.message);
  });

  test('wraps non-Error rejections without logging their content', async () => {
    fetchWalletStatus.mockRejectedValue('private rejection value');

    const { result } = renderHook(() => useMyAgentId());

    await waitFor(() => expect(result.current.status).toBe('error'));
    expect(result.current).toEqual({
      status: 'error',
      error: new Error('private rejection value'),
    });
    expect(debugLog.mock.calls.flat().join(' ')).not.toContain('private rejection value');
  });

  test('does not update state when resolution becomes stale after unmount', async () => {
    const request = deferred<ReturnType<typeof walletStatus>>();
    fetchWalletStatus.mockReturnValue(request.promise);

    const { unmount } = renderHook(() => useMyAgentId());
    unmount();

    await act(async () => {
      request.resolve(walletStatus([{ chain: 'solana', address: 'StaleAgent' }]));
      await request.promise;
    });

    expect(setState).not.toHaveBeenCalled();
  });

  test('does not log or update state when rejection becomes stale after unmount', async () => {
    const request = deferred<ReturnType<typeof walletStatus>>();
    fetchWalletStatus.mockReturnValue(request.promise);

    const { unmount } = renderHook(() => useMyAgentId());
    unmount();

    await act(async () => {
      request.reject(new Error('stale private wallet failure'));
      await request.promise.catch(() => undefined);
    });

    expect(debugLog).not.toHaveBeenCalled();
    expect(setState).not.toHaveBeenCalled();
  });
});
