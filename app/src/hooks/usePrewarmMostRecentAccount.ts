import { useEffect } from 'react';

import { prewarmWebviewAccount } from '../services/webviewAccountService';
import { selectLastActiveAccountId } from '../store/accountsSlice';
import { useAppSelector } from '../store/hooks';
import type { Account } from '../types/accounts';

/**
 * Cap on `accounts.length` for which the MRU prewarm runs. Power users
 * with many accounts skip prewarm so the spawn cost stays bounded — the
 * prewarmed webview reserves a CEF process + provider profile, and we
 * don't want a 20-account user to have all 20 warming on launch.
 */
export const PREWARM_MAX_ACCOUNTS = 5;

interface UsePrewarmMostRecentAccountArgs {
  accounts: Account[];
  accountsById: Record<string, Account | undefined>;
  activeAccountId: string | null;
}

/**
 * Issue #1233 — fire-and-forget prewarm of the most-recently-active account
 * once on mount of the Accounts page. The prewarmed webview is spawned
 * off-screen with the full handler / scanner / notification setup, so the
 * eventual user click hits the warm-reopen branch in
 * `webview_account_open` and emits `state:"reused"` instead of paying the
 * cold-load wait.
 *
 * The MRU id is read from the persisted Redux store
 * (`selectLastActiveAccountId`) — same single source of truth the rest of
 * Accounts uses, no separate `localStorage` channel.
 *
 * Skips when:
 *   - no MRU id in store (first run)
 *   - the user has more than `PREWARM_MAX_ACCOUNTS` accounts (bound the
 *     spawn cost on power users)
 *   - the MRU account is the currently active one (no point prewarming
 *     what's already on screen)
 *   - the MRU account is already pending / loading / open (live or
 *     in-flight)
 *
 * Runs exactly once per mount on purpose: the Tauri command itself is
 * idempotent server-side, but re-firing on every Redux churn would just
 * generate noise in the logs.
 */
export function usePrewarmMostRecentAccount({
  accounts,
  accountsById,
  activeAccountId,
}: UsePrewarmMostRecentAccountArgs): void {
  const mruId = useAppSelector(selectLastActiveAccountId);
  useEffect(() => {
    if (!mruId) return;
    if (accounts.length === 0 || accounts.length > PREWARM_MAX_ACCOUNTS) return;
    const acct = accountsById[mruId];
    if (!acct) return;
    if (acct.id === activeAccountId) return;
    if (acct.status === 'open' || acct.status === 'loading' || acct.status === 'pending') {
      return;
    }
    void prewarmWebviewAccount(acct.id, acct.provider);
    // Mount-only by design — see docstring. Snapshotting deps captured at
    // first render keeps the prewarm a single fire even when the parent
    // re-renders for unrelated reasons (resize, status flip on another
    // account, etc.). Rule isn't enforced in this repo's ESLint config so
    // the prose comment carries the intent.
  }, []);
}
