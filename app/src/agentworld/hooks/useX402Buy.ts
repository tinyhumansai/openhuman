/**
 * useX402Buy — shared confirm-before-spend state machine for Agent World x402
 * buy flows (products and identity listings).
 *
 * Parameterised by a buy function `(id, { confirmed }) => Promise<X402BuyResult>`
 * so the same two-call flow drives both `marketplace.buyProduct` and
 * `marketplace.buyIdentity`. The hook never spends on its own: `begin` probes
 * for the challenge (confirmed:false), `confirmPay` is the only call that runs
 * with confirmed:true.
 */
import { useState } from 'react';

import {
  PaymentRequiredError,
  type RegistrationChallenge,
  type RegistryWalletBalance,
  type X402BuyResult,
} from '../../lib/agentworld/invokeApiClient';

export type X402BuyState =
  | { phase: 'idle' }
  | { phase: 'challenge_loading' }
  | {
      phase: 'confirm';
      challenge: RegistrationChallenge;
      balance: RegistryWalletBalance | null;
      walletAddress: string;
    }
  | {
      phase: 'paying';
      challenge: RegistrationChallenge;
      balance: RegistryWalletBalance | null;
      walletAddress: string;
    }
  | { phase: 'success'; result: Record<string, unknown>; onChainTx?: string; network?: string }
  | { phase: 'error'; message: string; onChainTx?: string };

export type X402BuyFn = (id: string, opts: { confirmed: boolean }) => Promise<X402BuyResult>;

/** Devnet/mainnet Solana explorer link for a settled payment tx. */
export function explorerTxUrl(tx: string, network?: string): string {
  const cluster = (network ?? '').toLowerCase().includes('devnet') ? '?cluster=devnet' : '';
  return `https://explorer.solana.com/tx/${tx}${cluster}`;
}

/** Pull the broadcast tx out of a post-payment error string ("onChainTx=<sig>"). */
export function extractOnChainTx(message: string): string | undefined {
  const match = /onChainTx=([^)\s;]+)/.exec(message);
  return match?.[1];
}

export interface UseX402Buy {
  state: X402BuyState;
  reset: () => void;
  begin: (id: string) => void;
  confirmPay: (
    id: string,
    challenge: RegistrationChallenge,
    balance: RegistryWalletBalance | null,
    walletAddress: string
  ) => void;
}

export function useX402Buy(buyFn: X402BuyFn): UseX402Buy {
  const [state, setState] = useState<X402BuyState>({ phase: 'idle' });

  function reset() {
    setState({ phase: 'idle' });
  }

  // Phase A — probe for the challenge + balance (no spend).
  function begin(id: string) {
    setState({ phase: 'challenge_loading' });
    void buyFn(id, { confirmed: false })
      .then(res => {
        if (res.challenge) {
          setState({
            phase: 'confirm',
            challenge: res.challenge,
            balance: res.walletBalance ?? null,
            walletAddress: res.walletAddress ?? '',
          });
        } else if (res.result) {
          setState({ phase: 'success', result: res.result });
        } else {
          setState({ phase: 'error', message: 'Unexpected response from purchase.' });
        }
      })
      .catch((err: unknown) => {
        const message = err instanceof PaymentRequiredError ? 'Payment required.' : String(err);
        setState({ phase: 'error', message });
      });
  }

  // Phase B — pay on-chain + complete the purchase (spends). Carries the
  // confirm-phase balance + wallet through so the dialog keeps showing them.
  function confirmPay(
    id: string,
    challenge: RegistrationChallenge,
    balance: RegistryWalletBalance | null,
    walletAddress: string
  ) {
    setState({ phase: 'paying', challenge, balance, walletAddress });
    void buyFn(id, { confirmed: true })
      .then(res => {
        if (res.result) {
          setState({
            phase: 'success',
            result: res.result,
            onChainTx: res.payment?.onChainTx,
            network: challenge.network,
          });
        } else {
          setState({ phase: 'error', message: 'Purchase did not complete.' });
        }
      })
      .catch((err: unknown) => {
        const message = String(err);
        setState({ phase: 'error', message, onChainTx: extractOnChainTx(message) });
      });
  }

  return { state, reset, begin, confirmPay };
}
