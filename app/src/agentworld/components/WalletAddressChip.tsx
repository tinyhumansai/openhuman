/**
 * WalletAddressChip — persistent wallet address display for the Agent World
 * sidebar header.
 *
 * Fetches the wallet status on mount, extracts the Solana account address
 * (= tiny.place cryptoId), and renders a compact monospace chip with a
 * copy-to-clipboard button. Visible from every Agent World section without
 * requiring navigation.
 *
 * States:
 *   loading  — pulse skeleton while wallet_status is in-flight.
 *   ready    — truncated 6…4 address + copy button.
 *   locked   — muted "Wallet not set up" label (no copy).
 *
 * Copy pattern mirrors WalletBalancesPanel (2s checkmark feedback).
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { fetchWalletStatus } from '../../services/walletApi';

// ---------------------------------------------------------------------------
// Address helpers
// ---------------------------------------------------------------------------

/** Truncate a crypto address to first 6 + last 4: `AbCdEf…wxyz`. */
function truncateAddress(address: string): string {
  if (address.length <= 12) return address;
  return `${address.slice(0, 6)}…${address.slice(-4)}`;
}

// ---------------------------------------------------------------------------
// Icon primitives (inline SVG, 16×16, no external deps)
// ---------------------------------------------------------------------------

const WalletIcon = () => (
  <svg
    width="14"
    height="14"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth={2}
    strokeLinecap="round"
    strokeLinejoin="round"
    aria-hidden="true">
    <path d="M20 7H4a2 2 0 00-2 2v10a2 2 0 002 2h16a2 2 0 002-2V9a2 2 0 00-2-2z" />
    <path d="M16 3H8a2 2 0 00-2 2v2h12V5a2 2 0 00-2-2z" />
    <circle cx="16" cy="14" r="1" fill="currentColor" />
  </svg>
);

const CopyIcon = () => (
  <svg
    width="12"
    height="12"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth={2}
    strokeLinecap="round"
    strokeLinejoin="round"
    aria-hidden="true">
    <rect x="9" y="9" width="13" height="13" rx="2" ry="2" />
    <path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1" />
  </svg>
);

const CheckIcon = () => (
  <svg
    width="12"
    height="12"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth={2.5}
    strokeLinecap="round"
    strokeLinejoin="round"
    aria-hidden="true">
    <polyline points="20 6 9 17 4 12" />
  </svg>
);

// ---------------------------------------------------------------------------
// Component state type
// ---------------------------------------------------------------------------

type ChipState =
  | { status: 'loading' }
  | { status: 'ready'; address: string }
  | { status: 'locked' }
  | { status: 'error' };

// ---------------------------------------------------------------------------
// WalletAddressChip
// ---------------------------------------------------------------------------

export default function WalletAddressChip() {
  const { t } = useT();
  const [state, setState] = useState<ChipState>({ status: 'loading' });
  const [copied, setCopied] = useState(false);
  const copyResetTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Monotonic request id so a slow/aborted fetch can't clobber a newer one
  // (e.g. when the user taps "retry" while a prior request is still in flight).
  const latestRequestIdRef = useRef(0);

  // Fetch wallet status and resolve the chip state. Distinguishes the genuine
  // "not set up" case (a successful wallet_status with no Solana account) from a
  // transient RPC/transport failure — the latter must NOT masquerade as a locked
  // wallet for a user who already has one configured.
  const loadStatus = useCallback(async () => {
    const requestId = ++latestRequestIdRef.current;
    try {
      const status = await fetchWalletStatus();
      const solana = (status.accounts ?? []).find(a => a.chain === 'solana');
      if (requestId !== latestRequestIdRef.current) return;
      if (solana?.address) {
        setState({ status: 'ready', address: solana.address });
      } else {
        // Successful response, but the wallet has no Solana account yet.
        setState({ status: 'locked' });
      }
    } catch (err) {
      if (requestId !== latestRequestIdRef.current) return;
      // Core RPC unavailable / timeout / transport error — surface a retryable
      // error state rather than mislabelling a configured wallet as "not set up".
      const message = err instanceof Error ? err.message : String(err);
      console.debug('[walletAddressChip] wallet_status fetch failed:', message);
      setState({ status: 'error' });
    }
  }, []);

  // Fetch wallet status on mount.
  useEffect(() => {
    // loadStatus only calls setState after an awaited fetch (never synchronously),
    // so it does not cause the cascading renders this rule guards against.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void loadStatus();
    return () => {
      // Invalidate any in-flight request so its resolution is ignored.
      latestRequestIdRef.current += 1;
    };
  }, [loadStatus]);

  // Cleanup copy-reset timer on unmount.
  useEffect(
    () => () => {
      if (copyResetTimerRef.current !== null) {
        clearTimeout(copyResetTimerRef.current);
        copyResetTimerRef.current = null;
      }
    },
    []
  );

  const handleCopy = useCallback(async (address: string) => {
    try {
      await navigator.clipboard.writeText(address);
      setCopied(true);
      if (copyResetTimerRef.current !== null) {
        clearTimeout(copyResetTimerRef.current);
      }
      copyResetTimerRef.current = setTimeout(() => {
        setCopied(false);
        copyResetTimerRef.current = null;
      }, 2000);
    } catch {
      // Clipboard unavailable in this webview context; silently skip.
    }
  }, []);

  // ── Loading skeleton ─────────────────────────────────────────────────────
  if (state.status === 'loading') {
    return (
      <div data-testid="wallet-address-chip" className="flex items-center gap-1.5 animate-pulse">
        <div className="h-3.5 w-3.5 rounded bg-surface-strong shrink-0" />
        <div className="h-3 w-24 rounded bg-surface-strong" />
      </div>
    );
  }

  // ── Error — transient RPC/transport failure, offer a retry ────────────────
  if (state.status === 'error') {
    return (
      <button
        type="button"
        data-testid="wallet-address-chip"
        onClick={() => {
          // Show the loading skeleton again while the retry is in flight.
          setState({ status: 'loading' });
          void loadStatus();
        }}
        aria-label={t('agentWorld.walletRetry')}
        title={t('agentWorld.walletRetry')}
        className="flex items-center gap-1.5 text-content-faint transition-colors hover:text-content-secondary focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-primary-500">
        <WalletIcon />
        <span className="text-[11px] leading-none">{t('agentWorld.walletUnavailable')}</span>
      </button>
    );
  }

  // ── Locked / not configured ───────────────────────────────────────────────
  if (state.status === 'locked') {
    return (
      <div
        data-testid="wallet-address-chip"
        className="flex items-center gap-1.5 text-content-faint">
        <WalletIcon />
        <span className="text-[11px] leading-none">{t('agentWorld.walletNotConfigured')}</span>
      </div>
    );
  }

  // ── Ready — show truncated address + copy button ──────────────────────────
  const { address } = state;
  const truncated = truncateAddress(address);

  return (
    <div data-testid="wallet-address-chip" className="flex items-center gap-1.5 text-content-muted">
      <WalletIcon />
      <span className="font-mono text-[11px] leading-none tracking-tight" title={address}>
        {truncated}
      </span>
      <button
        type="button"
        aria-label={copied ? t('agentWorld.addressCopied') : t('agentWorld.copyAddress')}
        title={copied ? t('agentWorld.addressCopied') : t('agentWorld.copyAddress')}
        onClick={() => void handleCopy(address)}
        className="shrink-0 rounded p-0.5 text-content-faint transition-colors hover:text-content-secondary focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-primary-500">
        {copied ? <CheckIcon /> : <CopyIcon />}
      </button>
    </div>
  );
}
