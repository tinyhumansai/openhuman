import { renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { resolveWalletConfigured, useWalletConfigured } from './useWalletConfigured';

const fetchWalletStatus = vi.fn();
vi.mock('../services/walletApi', () => ({ fetchWalletStatus: () => fetchWalletStatus() }));

describe('wallet-configured gate (#5805)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe('resolveWalletConfigured', () => {
    it('reports `yes` when a wallet is configured', async () => {
      fetchWalletStatus.mockResolvedValue({ configured: true });
      await expect(resolveWalletConfigured()).resolves.toBe('yes');
    });

    it('reports `no` when wallet_status resolves with no wallet', async () => {
      fetchWalletStatus.mockResolvedValue({ configured: false });
      await expect(resolveWalletConfigured()).resolves.toBe('no');
    });

    // The load-bearing case: a failed status fetch must NOT be reported as
    // `no`. Callers skip their wallet-gated call only on a positive `no`, so
    // collapsing this into `no` would let a transport blip hide a wallet that
    // actually exists.
    it('reports `unknown` — never `no` — when the status fetch itself fails', async () => {
      fetchWalletStatus.mockRejectedValue(new Error('rpc transport failed'));
      await expect(resolveWalletConfigured()).resolves.toBe('unknown');
    });

    // It is awaited inline before a gated call, so a rejection here would
    // propagate into the caller it was meant to protect.
    it('never rejects', async () => {
      fetchWalletStatus.mockRejectedValue(new Error('boom'));
      await expect(resolveWalletConfigured()).resolves.toBeDefined();
    });
  });

  describe('useWalletConfigured', () => {
    it('starts at `resolving` so callers fire nothing before the answer', async () => {
      fetchWalletStatus.mockResolvedValue({ configured: true });
      const { result } = renderHook(() => useWalletConfigured());

      // Synchronously after mount, before the promise settles.
      expect(result.current).toBe('resolving');
      await waitFor(() => expect(result.current).toBe('yes'));
    });

    it('settles to `no` when no wallet is configured', async () => {
      fetchWalletStatus.mockResolvedValue({ configured: false });
      const { result } = renderHook(() => useWalletConfigured());

      await waitFor(() => expect(result.current).toBe('no'));
    });

    it('settles to `unknown` when the status fetch fails', async () => {
      fetchWalletStatus.mockRejectedValue(new Error('rpc transport failed'));
      const { result } = renderHook(() => useWalletConfigured());

      await waitFor(() => expect(result.current).toBe('unknown'));
    });

    // Unmounting before the probe settles must not set state on a dead
    // component — that is what the `cancelled` flag in the effect cleanup is
    // for, and it is only exercised if a test unmounts mid-flight.
    it('does not set state after unmount', async () => {
      let settle: (value: { configured: boolean }) => void = () => {};
      fetchWalletStatus.mockReturnValue(
        new Promise<{ configured: boolean }>(resolve => {
          settle = resolve;
        })
      );
      const errors: unknown[] = [];
      const spy = vi.spyOn(console, 'error').mockImplementation((...args) => errors.push(args));

      const { result, unmount } = renderHook(() => useWalletConfigured());
      expect(result.current).toBe('resolving');
      unmount();
      settle({ configured: true });
      await Promise.resolve();

      expect(errors).toHaveLength(0);
      spy.mockRestore();
    });
  });
});
