/**
 * Tests for useX402Buy — the shared x402 buy state machine + its helpers.
 * Drives the hook with a mocked buy function (no RPC). Generic placeholders only.
 */
import { act, renderHook, waitFor } from '@testing-library/react';
import { describe, expect, test, vi } from 'vitest';

import { PaymentRequiredError, type X402BuyResult } from '../../lib/agentworld/invokeApiClient';
import { explorerTxUrl, extractOnChainTx, useX402Buy } from './useX402Buy';

const CHALLENGE = { amount: '10000000', asset: 'USDC', network: 'solana-devnet' };
const BALANCE = { raw: '50000000', formatted: '50', decimals: 6, assetSymbol: 'USDC' };

describe('helpers', () => {
  test('explorerTxUrl appends the devnet cluster only for devnet networks', () => {
    expect(explorerTxUrl('Tx1', 'solana-devnet')).toBe(
      'https://explorer.solana.com/tx/Tx1?cluster=devnet'
    );
    expect(explorerTxUrl('Tx1', 'solana-mainnet')).toBe('https://explorer.solana.com/tx/Tx1');
    expect(explorerTxUrl('Tx1')).toBe('https://explorer.solana.com/tx/Tx1');
  });

  test('extractOnChainTx pulls the tx out of an error string', () => {
    expect(extractOnChainTx('paid but failed (onChainTx=Sig9); retry')).toBe('Sig9');
    expect(extractOnChainTx('no tx here')).toBeUndefined();
  });
});

describe('useX402Buy', () => {
  test('begin → confirm exposes the challenge + balance', async () => {
    const buyFn = vi
      .fn()
      .mockResolvedValue({
        challenge: CHALLENGE,
        walletBalance: BALANCE,
        walletAddress: 'Wallet1',
      } satisfies X402BuyResult);
    const { result } = renderHook(() => useX402Buy(buyFn));

    act(() => result.current.begin('id-1'));
    await waitFor(() => expect(result.current.state.phase).toBe('confirm'));
    expect(buyFn).toHaveBeenCalledWith('id-1', { confirmed: false });
    if (result.current.state.phase === 'confirm') {
      expect(result.current.state.balance).toEqual(BALANCE);
      expect(result.current.state.walletAddress).toBe('Wallet1');
    }
  });

  test('begin short-circuits to success when the result needs no payment', async () => {
    const buyFn = vi.fn().mockResolvedValue({ result: { saleId: 's1' } });
    const { result } = renderHook(() => useX402Buy(buyFn));
    act(() => result.current.begin('id-1'));
    await waitFor(() => expect(result.current.state.phase).toBe('success'));
  });

  test('begin with neither result nor challenge errors', async () => {
    const buyFn = vi.fn().mockResolvedValue({});
    const { result } = renderHook(() => useX402Buy(buyFn));
    act(() => result.current.begin('id-1'));
    await waitFor(() => expect(result.current.state.phase).toBe('error'));
    if (result.current.state.phase === 'error') {
      expect(result.current.state.message).toMatch(/Unexpected/);
    }
  });

  test('begin maps a PaymentRequiredError to a payment notice', async () => {
    const buyFn = vi.fn().mockRejectedValue(new PaymentRequiredError({ t: 1 }));
    const { result } = renderHook(() => useX402Buy(buyFn));
    act(() => result.current.begin('id-1'));
    await waitFor(() => expect(result.current.state.phase).toBe('error'));
    if (result.current.state.phase === 'error') {
      expect(result.current.state.message).toBe('Payment required.');
    }
  });

  test('confirmPay success carries the on-chain tx + network', async () => {
    const buyFn = vi
      .fn()
      .mockResolvedValueOnce({ challenge: CHALLENGE, walletBalance: BALANCE, walletAddress: 'W' })
      .mockResolvedValueOnce({ result: { saleId: 's1' }, payment: { onChainTx: 'TxOK' } });
    const { result } = renderHook(() => useX402Buy(buyFn));
    act(() => result.current.begin('id-1'));
    await waitFor(() => expect(result.current.state.phase).toBe('confirm'));
    act(() => result.current.confirmPay('id-1', CHALLENGE, BALANCE, 'W'));
    await waitFor(() => expect(result.current.state.phase).toBe('success'));
    expect(buyFn).toHaveBeenLastCalledWith('id-1', { confirmed: true });
    if (result.current.state.phase === 'success') {
      expect(result.current.state.onChainTx).toBe('TxOK');
      expect(result.current.state.network).toBe('solana-devnet');
    }
  });

  test('confirmPay with no result errors', async () => {
    const buyFn = vi.fn().mockResolvedValue({});
    const { result } = renderHook(() => useX402Buy(buyFn));
    act(() => result.current.confirmPay('id-1', CHALLENGE, BALANCE, 'W'));
    await waitFor(() => expect(result.current.state.phase).toBe('error'));
    if (result.current.state.phase === 'error') {
      expect(result.current.state.message).toMatch(/did not complete/);
    }
  });

  test('confirmPay failure extracts the broadcast tx from the error', async () => {
    const buyFn = vi.fn().mockRejectedValue(new Error('paid (onChainTx=BrokeTx)'));
    const { result } = renderHook(() => useX402Buy(buyFn));
    act(() => result.current.confirmPay('id-1', CHALLENGE, BALANCE, 'W'));
    await waitFor(() => expect(result.current.state.phase).toBe('error'));
    if (result.current.state.phase === 'error') {
      expect(result.current.state.onChainTx).toBe('BrokeTx');
    }
  });

  test('reset returns to idle', async () => {
    const buyFn = vi.fn().mockResolvedValue({ result: { saleId: 's1' } });
    const { result } = renderHook(() => useX402Buy(buyFn));
    act(() => result.current.begin('id-1'));
    await waitFor(() => expect(result.current.state.phase).toBe('success'));
    act(() => result.current.reset());
    expect(result.current.state.phase).toBe('idle');
  });
});
