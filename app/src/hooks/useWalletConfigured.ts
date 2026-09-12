/**
 * Shared wallet-configured gate for wallet-requiring RPCs.
 *
 * ## Why this exists
 *
 * Several core RPCs require the local wallet before they can run
 * (EVM signing, balances, deBridge quotes). When no
 * wallet is set up they reject with the core's `WALLET_NOT_CONFIGURED_MESSAGE`.
 * That is an *expected* state — a wallet is optional and most users never
 * create one — but it still travels as an `Err`, so every such call becomes an
 * error the boundary has to classify.
 *
 * The core-side classifier (`ExpectedErrorKind::WalletNotConfigured`) keeps
 * those out of Sentry, and it is deliberately kept as defense-in-depth. This
 * hook is the other half: **do not make the call at all when we have a positive
 * signal that no wallet exists.** `openhuman.wallet_status` answers the same
 * question and resolves normally for a wallet-less user, so the gate costs one
 * non-erroring RPC and removes an erroring one.
 *
 * This pairs prevention-at-source with boundary classification so the next
 * wallet-gated caller inherits the guard instead of re-deriving it.
 *
 * ## The `unknown` state is load-bearing
 *
 * Gate only on a **positive** "no wallet". If `wallet_status` itself fails we
 * cannot prove the wallet is absent, so we return `unknown` and the caller
 * proceeds — a transport blip must not render a configured wallet as missing.
 * Any wallet error that follows is then handled by the boundary classifier.
 *
 * ## Deliberately not cached
 *
 * Resolution runs per mount. A user who sets up a wallet mid-session must be
 * picked up on the next mount, and a cached `no` would strand them until
 * restart. `wallet_status` is a cheap local RPC that does not error, so there
 * is nothing to save by caching it.
 */
import { useEffect, useState } from 'react';

import { fetchWalletStatus } from '../services/walletApi';

/**
 * - `resolving` — `wallet_status` still in flight. Callers must NOT fire
 *   wallet-requiring RPCs yet.
 * - `no` — resolved, and no wallet is configured. The only state that carries a
 *   positive lever to skip a wallet-gated call.
 * - `yes` — a wallet is configured; proceed.
 * - `unknown` — the `wallet_status` fetch itself failed. Proceed, and let the
 *   boundary classifier handle any wallet error that follows.
 */
export type WalletConfigured = 'resolving' | 'no' | 'yes' | 'unknown';

/**
 * Resolve the wallet-configured state once, outside React.
 *
 * Exposed for callers that are not components. Never rejects: a failed status
 * fetch becomes `unknown`.
 */
export async function resolveWalletConfigured(): Promise<Exclude<WalletConfigured, 'resolving'>> {
  try {
    const status = await fetchWalletStatus();
    return status.configured ? 'yes' : 'no';
  } catch {
    // Cannot prove absence — see "The `unknown` state is load-bearing" above.
    return 'unknown';
  }
}

/** React binding over {@link resolveWalletConfigured}; `resolving` until settled. */
export function useWalletConfigured(): WalletConfigured {
  const [configured, setConfigured] = useState<WalletConfigured>('resolving');
  useEffect(() => {
    let cancelled = false;
    void resolveWalletConfigured().then(next => {
      if (!cancelled) setConfigured(next);
    });
    return () => {
      cancelled = true;
    };
  }, []);
  return configured;
}
