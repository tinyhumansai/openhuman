/**
 * IdentitiesSection — Agent World Identities section.
 *
 * Provides three tabs (Register / Registry / Trading) mirroring the structure
 * of the tiny.place Identities screen. All data is fetched through the
 * OpenHuman core RPC bridge (invokeApiClient) — no direct tiny.place HTTP calls
 * or @tanstack/react-query usage in this file.
 *
 * Write flows are live x402:
 * - Register tab: confirm-before-spend registration (pay on-chain → register).
 * - Trading tab: Buy a fixed-price listing (confirm-before-spend), plus Bid /
 *   Offer commitments (signed authorizations — funds move only on acceptance).
 * Money only moves after the user confirms. The read-only data views (Registry
 * listing, floor prices, recent sales) are fully functional.
 */
import { useCallback, useEffect, useReducer, useRef, useState } from 'react';

import PanelScaffold from '../../components/layout/PanelScaffold';
import {
  type AvailabilityResponse,
  type DirectoryIdentityListingsResponse,
  type IdentitiesResponse,
  type IdentityFloor,
  type IdentityListing,
  PaymentRequiredError,
  type RecentSalesResponse,
  type RegisteredIdentity,
  type RegistrationChallenge,
  type RegistryWalletBalance,
} from '../../lib/agentworld/invokeApiClient';
import { apiClient } from '../AgentWorldShell';
import AmountCommitDialog from '../components/AmountCommitDialog';
import X402ConfirmDialog from '../components/X402ConfirmDialog';
import { explorerTxUrl as buyExplorerTxUrl, useX402Buy } from '../hooks/useX402Buy';

// ── Types ─────────────────────────────────────────────────────────────────────

type Tab = 'register' | 'registry' | 'trading';

// Generic async state for a single fetch
type AsyncState<T> =
  | { status: 'idle' }
  | { status: 'loading' }
  | { status: 'payment_required'; challenge: unknown }
  | { status: 'error'; message: string }
  | { status: 'ok'; data: T };

const MARKETPLACE_PAGE_SIZE = 50;
const MARKETPLACE_TARGET_ACTIVE = 50;
const MARKETPLACE_MAX_PAGES = 5;
const FLOOR_PAGE_SIZE = 20;
const FLOOR_MAX_PAGES = 10;

function isActiveListing(identity: IdentityListing): boolean {
  return identity.status == null || identity.status === 'active';
}

async function fetchActiveMarketplaceIdentities(): Promise<IdentitiesResponse> {
  const active: Array<IdentityListing> = [];
  let lastResponse: IdentitiesResponse | null = null;

  for (
    let page = 0;
    page < MARKETPLACE_MAX_PAGES && active.length < MARKETPLACE_TARGET_ACTIVE;
    page++
  ) {
    const response = await apiClient.graphql.identityListings({
      limit: MARKETPLACE_PAGE_SIZE,
      offset: page * MARKETPLACE_PAGE_SIZE,
    });
    lastResponse = response;
    const identities = response.identities ?? [];
    active.push(...identities.filter(isActiveListing));
    if (identities.length < MARKETPLACE_PAGE_SIZE) {
      break;
    }
  }

  return {
    ...(lastResponse ?? { identities: [] }),
    identities: active.slice(0, MARKETPLACE_TARGET_ACTIVE),
    count: active.length,
  };
}

async function fetchActiveFloorPrice(length: number): Promise<IdentityFloor> {
  for (let page = 0; page < FLOOR_MAX_PAGES; page++) {
    const response = await apiClient.graphql.identityListings({
      length,
      limit: FLOOR_PAGE_SIZE,
      offset: page * FLOOR_PAGE_SIZE,
      sortBy: 'price_asc',
    });
    const identities = response.identities ?? [];
    const listing = identities.find(isActiveListing);
    if (listing) {
      return { length, price: listing.price };
    }
    if (identities.length < FLOOR_PAGE_SIZE) {
      break;
    }
  }

  return { length, price: undefined };
}

// ── Small hooks ───────────────────────────────────────────────────────────────

function useHandleAvailability(
  handle: string
): AsyncState<AvailabilityResponse> & { check: () => void } {
  const [state, setState] = useState<AsyncState<AvailabilityResponse>>({ status: 'idle' });
  const abortRef = useRef<AbortController | null>(null);

  function check() {
    const normalized = handle.trim().replace(/^@+/, '');
    if (!normalized) return;
    if (abortRef.current) abortRef.current.abort();
    const ctrl = new AbortController();
    abortRef.current = ctrl;
    setState({ status: 'loading' });
    void apiClient.registry
      .get(`@${normalized}`)
      .then(data => {
        if (!ctrl.signal.aborted) setState({ status: 'ok', data });
      })
      .catch((err: unknown) => {
        if (ctrl.signal.aborted) return;
        if (err instanceof PaymentRequiredError) {
          setState({ status: 'payment_required', challenge: err.challenge });
        } else {
          setState({ status: 'error', message: String(err) });
        }
      });
  }

  return { ...state, check };
}

function useMarketplaceIdentities(): AsyncState<IdentitiesResponse> {
  const [state, setState] = useState<AsyncState<IdentitiesResponse>>({ status: 'loading' });
  useEffect(() => {
    let cancelled = false;
    void fetchActiveMarketplaceIdentities()
      .then(data => {
        if (!cancelled) {
          setState({ status: 'ok', data });
        }
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        if (err instanceof PaymentRequiredError) {
          setState({ status: 'payment_required', challenge: err.challenge });
        } else {
          setState({ status: 'error', message: String(err) });
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);
  return state;
}

function useDirectoryIdentities(): AsyncState<DirectoryIdentityListingsResponse> {
  const [state, setState] = useState<AsyncState<DirectoryIdentityListingsResponse>>({
    status: 'loading',
  });
  useEffect(() => {
    let cancelled = false;
    void apiClient.directoryIdentities
      .list({ limit: 20 })
      .then(data => {
        if (!cancelled) setState({ status: 'ok', data });
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        if (err instanceof PaymentRequiredError) {
          setState({ status: 'payment_required', challenge: err.challenge });
        } else {
          setState({ status: 'error', message: String(err) });
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);
  return state;
}

function useFloorPrice(length: number): AsyncState<IdentityFloor> {
  const [state, setState] = useState<AsyncState<IdentityFloor>>({ status: 'loading' });
  useEffect(() => {
    let cancelled = false;
    void fetchActiveFloorPrice(length)
      .then(data => {
        if (cancelled) return;
        setState({ status: 'ok', data });
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        setState({ status: 'error', message: String(err) });
      });
    return () => {
      cancelled = true;
    };
  }, [length]);
  return state;
}

function useRecentSales(): AsyncState<RecentSalesResponse> {
  const [state, setState] = useState<AsyncState<RecentSalesResponse>>({ status: 'loading' });
  useEffect(() => {
    let cancelled = false;
    void apiClient.marketplace
      .recent()
      .then(data => {
        if (!cancelled) setState({ status: 'ok', data });
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        setState({ status: 'error', message: String(err) });
      });
    return () => {
      cancelled = true;
    };
  }, []);
  return state;
}

// ── Sub-components ────────────────────────────────────────────────────────────

function PaymentRequiredBanner() {
  return (
    <div className="flex flex-col items-center justify-center h-32 gap-2 text-amber-400">
      <p className="text-sm font-medium">Access requires payment</p>
      <p className="text-xs text-stone-500 dark:text-neutral-400">
        Your wallet will be used to fulfill the x402 payment challenge.
      </p>
    </div>
  );
}

function ErrorBanner({ message }: { message: string }) {
  const isWalletLocked =
    message.includes('wallet is not configured') ||
    message.includes('wallet secret material is missing');

  if (isWalletLocked) {
    return (
      <div className="flex flex-col items-center justify-center h-32 gap-2 text-stone-500 dark:text-neutral-400">
        <p className="text-sm font-medium">Unlock your wallet to use Agent World</p>
        <p className="text-xs">
          Agent World uses your wallet identity. Import your recovery phrase in Settings to
          continue.
        </p>
      </div>
    );
  }

  return (
    <div className="flex flex-col items-center justify-center h-32 gap-2 text-red-400">
      <p className="text-sm font-medium">Failed to load</p>
      <p className="text-xs text-stone-400 dark:text-neutral-500">{message}</p>
    </div>
  );
}

// Formats price amount + asset for display
function formatPrice(amount: string, asset: string): string {
  return `${amount} ${asset}`;
}

// ── Register tab ──────────────────────────────────────────────────────────────

// Devnet/mainnet Solana explorer link for a settled payment tx.
function explorerTxUrl(tx: string, network?: string): string {
  const cluster = (network ?? '').toLowerCase().includes('devnet') ? '?cluster=devnet' : '';
  return `https://explorer.solana.com/tx/${tx}${cluster}`;
}

// The Rust handler embeds "onChainTx=<sig>" in post-payment error strings so the
// UI can still surface the broadcast tx on a failed registration.
function extractOnChainTx(message: string): string | undefined {
  const match = /onChainTx=([^)\s;]+)/.exec(message);
  return match?.[1];
}

type RegState =
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
  | { phase: 'success'; identity: RegisteredIdentity; onChainTx?: string; network?: string }
  | { phase: 'error'; message: string; onChainTx?: string };

function useRegistration() {
  const [state, setState] = useState<RegState>({ phase: 'idle' });

  function reset() {
    setState({ phase: 'idle' });
  }

  // Phase A — fetch the challenge + balance (no spend).
  function begin(username: string) {
    setState({ phase: 'challenge_loading' });
    void apiClient.registry
      .register({ username, confirmed: false })
      .then(result => {
        if (result.identity) {
          setState({ phase: 'success', identity: result.identity });
        } else if (result.challenge) {
          setState({
            phase: 'confirm',
            challenge: result.challenge,
            balance: result.walletBalance ?? null,
            walletAddress: result.walletAddress ?? '',
          });
        } else {
          setState({ phase: 'error', message: 'Unexpected response from registration.' });
        }
      })
      .catch((err: unknown) => {
        const message = err instanceof PaymentRequiredError ? 'Payment required.' : String(err);
        setState({ phase: 'error', message });
      });
  }

  // Phase B — pay on-chain + register (spends). Carries the confirm-phase
  // balance + wallet through so the dialog keeps showing them while paying.
  function confirmPay(
    username: string,
    challenge: RegistrationChallenge,
    balance: RegistryWalletBalance | null,
    walletAddress: string
  ) {
    setState({ phase: 'paying', challenge, balance, walletAddress });
    void apiClient.registry
      .register({ username, confirmed: true, actorType: 'human', primary: true })
      .then(result => {
        if (result.identity) {
          setState({
            phase: 'success',
            identity: result.identity,
            onChainTx: result.payment?.onChainTx,
            network: challenge.network,
          });
        } else {
          setState({ phase: 'error', message: 'Registration did not complete.' });
        }
      })
      .catch((err: unknown) => {
        const message = String(err);
        setState({ phase: 'error', message, onChainTx: extractOnChainTx(message) });
      });
  }

  return { state, reset, begin, confirmPay };
}

function RegisterTab({ onRegistered }: { onRegistered?: () => void }) {
  const [input, setInput] = useState('');
  // sanitize: lowercase a-z, digits, _ only (mirrors tiny.place sanitizeHandle)
  function sanitize(value: string): string {
    return value.toLowerCase().replace(/[^a-z0-9_]/g, '');
  }

  const { check, ...availState } = useHandleAvailability(input);
  const reg = useRegistration();

  // Notify the parent once when registration transitions to success so it can
  // trigger a directory refetch (makes the new card immediately discoverable).
  useEffect(() => {
    if (reg.state.phase === 'success') {
      onRegistered?.();
    }
  }, [reg.state.phase, onRegistered]);

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    reg.reset();
    check();
  }

  // The available handle (normalized, no @) currently confirmed by availState.
  const availableHandle =
    availState.status === 'ok' && availState.data.available
      ? availState.data.name.replace(/^@+/, '')
      : null;

  const busy = reg.state.phase === 'challenge_loading' || reg.state.phase === 'paying';
  // Narrowed view for the confirm dialog (preserves the discriminated union
  // through the JSX `&&` chain below).
  const dialogState =
    reg.state.phase === 'confirm' || reg.state.phase === 'paying' ? reg.state : null;

  return (
    <div className="space-y-4">
      <div className="rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-900/50 p-4">
        <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100 mb-2">
          Check handle availability
        </h3>
        <form className="flex gap-2" onSubmit={handleSubmit}>
          <input
            className="flex-1 rounded-md border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder-stone-400 dark:placeholder-neutral-500 outline-none focus:border-primary-500"
            placeholder="Search for a name..."
            type="text"
            value={input}
            onChange={e => {
              setInput(sanitize(e.target.value));
            }}
          />
          <button
            type="submit"
            disabled={!input.trim()}
            className="rounded-md bg-primary-600 px-4 py-2 text-sm font-medium text-white disabled:opacity-50">
            Check
          </button>
        </form>

        {availState.status === 'loading' && (
          <p className="mt-2 text-xs text-stone-500 dark:text-neutral-400 animate-pulse">
            Checking…
          </p>
        )}
        {availState.status === 'payment_required' && (
          <p className="mt-2 text-xs text-amber-400">Payment required to check availability.</p>
        )}
        {availState.status === 'error' && (
          <p className="mt-2 text-xs text-red-400">{availState.message}</p>
        )}
        {availState.status === 'ok' && (
          <div className="mt-3">
            {availableHandle ? (
              <div className="flex items-center justify-between gap-2">
                <span className="text-xs font-medium text-green-500">
                  @{availableHandle} is available
                </span>
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => {
                    reg.begin(availableHandle);
                  }}
                  className="rounded-md bg-primary-600 px-3 py-1.5 text-xs font-medium text-white disabled:opacity-50">
                  {reg.state.phase === 'challenge_loading'
                    ? 'Loading…'
                    : `Register @${availableHandle}`}
                </button>
              </div>
            ) : (
              <div>
                <span className="text-xs font-medium text-red-500">
                  @{availState.data.name.replace(/^@+/, '')} is taken
                </span>
                {availState.data.identity && (
                  <span className="ml-2 text-xs text-stone-500 dark:text-neutral-400 font-mono">
                    {availState.data.identity.cryptoId.slice(0, 12)}...
                  </span>
                )}
              </div>
            )}
          </div>
        )}

        {/* Registration outcome surfaces inline below the availability row. */}
        {reg.state.phase === 'success' && (
          <div
            className="mt-3 rounded-md border border-green-500/30 bg-green-500/10 p-3"
            data-testid="register-success">
            <p className="text-xs font-medium text-green-500">
              Registered @{reg.state.identity.username?.replace(/^@+/, '') ?? availableHandle}
            </p>
            {reg.state.onChainTx && (
              <a
                href={explorerTxUrl(reg.state.onChainTx, reg.state.network)}
                target="_blank"
                rel="noreferrer"
                className="mt-1 inline-block text-xs text-primary-500 underline">
                View payment on Solana Explorer
              </a>
            )}
          </div>
        )}
        {reg.state.phase === 'error' && (
          <div
            className="mt-3 rounded-md border border-red-500/30 bg-red-500/10 p-3"
            data-testid="register-error">
            <p className="text-xs font-medium text-red-500">
              {reg.state.onChainTx
                ? 'Payment sent but registration did not complete.'
                : 'Registration failed.'}
            </p>
            <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400">{reg.state.message}</p>
            {reg.state.onChainTx && (
              <a
                href={explorerTxUrl(reg.state.onChainTx)}
                target="_blank"
                rel="noreferrer"
                className="mt-1 inline-block text-xs text-primary-500 underline">
                View payment on Solana Explorer
              </a>
            )}
          </div>
        )}
      </div>

      <div className="rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-900/50 p-4">
        <h4 className="text-xs font-semibold text-stone-900 dark:text-neutral-100 mb-2">
          Pricing tiers
        </h4>
        <div className="space-y-1">
          {[
            { label: '3 chars', example: '@abc', fee: '$250/yr' },
            { label: '4 chars', example: '@abcd', fee: '$50/yr' },
            { label: '5+ chars', example: '@abcde', fee: '$10/yr' },
          ].map(tier => (
            <div
              key={tier.label}
              className="flex items-center justify-between text-xs text-stone-500 dark:text-neutral-400">
              <span>
                {tier.label} <span className="font-mono opacity-60">({tier.example})</span>
              </span>
              <span className="font-medium">{tier.fee}</span>
            </div>
          ))}
        </div>
      </div>

      {/* Confirm-before-spend dialog (only while a challenge is pending). */}
      {dialogState && availableHandle && (
        <X402ConfirmDialog
          title={`Register @${availableHandle}`}
          subtitle="Confirm the x402 payment to claim this handle."
          amount={dialogState.challenge.amount ?? '0'}
          asset={dialogState.challenge.asset ?? 'USDC'}
          network={dialogState.challenge.network}
          balance={dialogState.balance}
          walletAddress={dialogState.walletAddress}
          busy={dialogState.phase === 'paying'}
          busyLabel="Paying…"
          onCancel={reg.reset}
          onConfirm={() => {
            reg.confirmPay(
              availableHandle,
              dialogState.challenge,
              dialogState.balance,
              dialogState.walletAddress
            );
          }}
        />
      )}
    </div>
  );
}

// ── Registry tab ──────────────────────────────────────────────────────────────

function RegistryTab() {
  const directoryState = useDirectoryIdentities();
  const listings = directoryState.status === 'ok' ? (directoryState.data.identities ?? []) : [];

  function formatDate(value: string): string {
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) return value;
    return date.toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' });
  }

  return (
    <div className="space-y-3">
      <div className="overflow-hidden rounded-lg border border-stone-200 dark:border-neutral-800">
        <div className="flex items-center justify-between border-b border-stone-200 dark:border-neutral-800 px-3 py-2">
          <span className="text-xs font-medium text-stone-900 dark:text-neutral-100">
            Directory identities
          </span>
          <span className="text-xs text-stone-400 dark:text-neutral-500">Live from staging</span>
        </div>

        {directoryState.status === 'loading' && (
          <p className="px-3 py-4 text-xs text-stone-500 dark:text-neutral-400 animate-pulse">
            Loading identities…
          </p>
        )}
        {directoryState.status === 'payment_required' && <PaymentRequiredBanner />}
        {directoryState.status === 'error' && <ErrorBanner message={directoryState.message} />}
        {directoryState.status === 'ok' && listings.length === 0 && (
          <p className="px-3 py-4 text-xs text-stone-500 dark:text-neutral-400">
            No directory identities are currently listed.
          </p>
        )}
        {listings.length > 0 && (
          <table className="w-full text-left text-xs">
            <thead>
              <tr className="border-b border-stone-200 dark:border-neutral-800">
                <th className="px-3 py-2 font-medium text-stone-400 dark:text-neutral-500">
                  Handle
                </th>
                <th className="px-3 py-2 font-medium text-stone-400 dark:text-neutral-500">
                  Seller
                </th>
                <th className="px-3 py-2 font-medium text-stone-400 dark:text-neutral-500">
                  Updated
                </th>
                <th className="px-3 py-2 font-medium text-stone-400 dark:text-neutral-500">
                  Status
                </th>
                <th className="px-3 py-2 text-right font-medium text-stone-400 dark:text-neutral-500">
                  Price
                </th>
              </tr>
            </thead>
            <tbody>
              {listings.map((entry, index) => (
                <tr
                  key={entry.listingId}
                  className={`border-b border-stone-200 dark:border-neutral-800 last:border-b-0 ${
                    index % 2 === 1 ? 'bg-stone-50 dark:bg-neutral-900/50' : ''
                  }`}>
                  <td className="px-3 py-2 font-medium text-stone-900 dark:text-neutral-100">
                    {entry.name}
                  </td>
                  <td className="px-3 py-2 font-mono text-stone-400 dark:text-neutral-500">
                    {entry.seller ?? '—'}
                  </td>
                  <td className="px-3 py-2 text-stone-400 dark:text-neutral-500">
                    {formatDate(entry.updatedAt)}
                  </td>
                  <td className="px-3 py-2">
                    <span
                      className={`rounded-full px-2 py-0.5 text-xs font-medium ${
                        entry.status === 'active'
                          ? 'bg-green-500/10 text-green-500'
                          : 'bg-amber-500/10 text-amber-500'
                      }`}>
                      {entry.status ?? 'unknown'}
                    </span>
                  </td>
                  <td className="px-3 py-2 text-right font-medium text-stone-900 dark:text-neutral-100">
                    {entry.price ? formatPrice(entry.price.amount, entry.price.asset) : '—'}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}

// ── Floor price card ──────────────────────────────────────────────────────────

function FloorCard({ length }: { length: number }) {
  const state = useFloorPrice(length);
  const labels: Record<number, string> = { 3: '3 chars', 4: '4 chars', 5: '5+ chars' };
  const descriptions: Record<number, string> = {
    3: 'Short handles',
    4: 'Compact handles',
    5: 'Long-form identities',
  };

  return (
    <div className="rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-900/50 p-3">
      <div className="text-xs text-stone-400 dark:text-neutral-500">
        {labels[length] ?? `${String(length)} chars`}
      </div>
      <div className="mt-1 text-sm font-semibold text-stone-900 dark:text-neutral-100">
        {state.status === 'loading' && (
          <span className="animate-pulse text-stone-400 dark:text-neutral-500">Loading…</span>
        )}
        {state.status === 'ok' && state.data.price
          ? formatPrice(state.data.price.amount, state.data.price.asset)
          : state.status === 'ok'
            ? 'No floor'
            : null}
        {state.status === 'error' && (
          <span className="text-stone-400 dark:text-neutral-500">Unavailable</span>
        )}
      </div>
      <div className="mt-1 text-xs text-stone-400 dark:text-neutral-500">
        {descriptions[length] ?? 'Handle identities'}
      </div>
    </div>
  );
}

// ── Trading tab ───────────────────────────────────────────────────────────────

function TradingTab() {
  const marketState = useMarketplaceIdentities();
  const salesState = useRecentSales();

  const listings = marketState.status === 'ok' ? (marketState.data.identities ?? []) : [];
  const sales = salesState.status === 'ok' ? (salesState.data.sales ?? []) : [];

  // x402 buy flow for fixed-price identity listings.
  const [buying, setBuying] = useState<IdentityListing | null>(null);
  const buy = useX402Buy((id, opts) => apiClient.marketplace.buyIdentity(id, opts));
  const bs = buy.state;
  function startBuy(listing: IdentityListing) {
    setBuying(listing);
    buy.begin(listing.listingId);
  }
  function closeBuy() {
    buy.reset();
    setBuying(null);
  }

  // x402 commitment flow (bid / offer) — no immediate spend.
  const [commit, setCommit] = useState<{ kind: 'bid' | 'offer'; listing: IdentityListing } | null>(
    null
  );
  const [commitState, setCommitState] = useState<{
    phase: 'idle' | 'busy' | 'success' | 'error';
    message?: string;
  }>({ phase: 'idle' });

  function closeCommit() {
    setCommit(null);
    setCommitState({ phase: 'idle' });
  }

  function submitCommit(amount: string) {
    if (!commit) return;
    const { kind, listing } = commit;
    const price = { amount, asset: listing.price.asset, network: listing.price.network ?? '' };
    setCommitState({ phase: 'busy' });
    const call =
      kind === 'bid'
        ? apiClient.marketplace.bid(listing.listingId, price)
        : apiClient.marketplace.offer(listing.name, price);
    void call
      .then(() => {
        setCommit(null);
        setCommitState({ phase: 'success' });
      })
      .catch((err: unknown) => {
        setCommitState({ phase: 'error', message: String(err) });
      });
  }

  return (
    <div className="space-y-4">
      {/* Floor prices */}
      <div>
        <h3 className="mb-2 text-xs font-semibold uppercase tracking-wider text-stone-400 dark:text-neutral-500">
          Floor Prices
        </h3>
        <div className="grid grid-cols-3 gap-2">
          {([3, 4, 5] as const).map(length => (
            <FloorCard key={length} length={length} />
          ))}
        </div>
      </div>

      {/* Listed for sale */}
      <div>
        <h3 className="mb-2 text-xs font-semibold uppercase tracking-wider text-stone-400 dark:text-neutral-500">
          Listed for Sale
        </h3>
        {marketState.status === 'loading' && (
          <p className="text-xs text-stone-500 dark:text-neutral-400 animate-pulse">
            Loading listings…
          </p>
        )}
        {marketState.status === 'payment_required' && <PaymentRequiredBanner />}
        {marketState.status === 'error' && <ErrorBanner message={marketState.message} />}
        {marketState.status === 'ok' && listings.length === 0 && (
          <p className="text-xs text-stone-500 dark:text-neutral-400">
            No identities listed for sale
          </p>
        )}
        {listings.length > 0 && (
          <div className="grid grid-cols-2 gap-2">
            {listings.map(listing => (
              <div
                key={listing.listingId}
                className="rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-900/50 p-3">
                <div className="flex items-center gap-2">
                  <span className="text-sm font-medium text-stone-900 dark:text-neutral-100">
                    {listing.name}
                  </span>
                  {listing.listingType === 'auction' && (
                    <span className="rounded-full bg-orange-600/20 px-2 py-0.5 text-xs font-medium text-orange-500">
                      Auction
                    </span>
                  )}
                </div>
                <div className="mt-1 text-xs font-semibold text-stone-900 dark:text-neutral-100">
                  {formatPrice(listing.price.amount, listing.price.asset)}
                </div>
                {listing.seller && (
                  <div className="mt-0.5 text-xs text-stone-400 dark:text-neutral-500">
                    by {listing.seller}
                  </div>
                )}
                <div className="mt-2 flex gap-1">
                  {listing.listingType !== 'auction' && (
                    <button
                      type="button"
                      disabled={buying !== null}
                      onClick={() => startBuy(listing)}
                      className="flex-1 rounded-md bg-primary-600 px-2 py-1 text-xs font-medium text-white disabled:opacity-50">
                      Buy
                    </button>
                  )}
                  {listing.listingType === 'auction' && (
                    <button
                      type="button"
                      disabled={commit !== null}
                      onClick={() => setCommit({ kind: 'bid', listing })}
                      className="flex-1 rounded-md bg-primary-600 px-2 py-1 text-xs font-medium text-white disabled:opacity-50">
                      Bid
                    </button>
                  )}
                  <button
                    type="button"
                    disabled={commit !== null}
                    onClick={() => setCommit({ kind: 'offer', listing })}
                    className="flex-1 rounded-md border border-stone-300 px-2 py-1 text-xs font-medium text-stone-700 disabled:opacity-50 dark:border-neutral-700 dark:text-neutral-200">
                    Offer
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}

        {/* Commitment (bid/offer) outcome banner. */}
        {commitState.phase === 'success' && (
          <div
            className="mt-3 rounded-md border border-green-500/30 bg-green-500/10 p-3"
            data-testid="commit-success">
            <p className="text-xs font-medium text-green-500">Commitment submitted.</p>
          </div>
        )}
        {commitState.phase === 'error' && (
          <div
            className="mt-3 rounded-md border border-red-500/30 bg-red-500/10 p-3"
            data-testid="commit-error">
            <p className="text-xs font-medium text-red-500">Commitment failed.</p>
            <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400">
              {commitState.message}
            </p>
          </div>
        )}

        {/* Bid / offer amount dialog. */}
        {commit && (
          <AmountCommitDialog
            title={
              commit.kind === 'bid'
                ? `Bid on ${commit.listing.name}`
                : `Offer for ${commit.listing.name}`
            }
            subtitle="A signed commitment — funds move only if it is accepted."
            asset={commit.listing.price.asset}
            submitLabel={commit.kind === 'bid' ? 'Place bid' : 'Submit offer'}
            busy={commitState.phase === 'busy'}
            onCancel={closeCommit}
            onSubmit={submitCommit}
          />
        )}

        {/* Buy outcome banner. */}
        {buying && bs.phase === 'success' && (
          <div
            className="mt-3 rounded-md border border-green-500/30 bg-green-500/10 p-3"
            data-testid="buy-identity-success">
            <p className="text-xs font-medium text-green-500">Purchased {buying.name}</p>
            {bs.onChainTx && (
              <a
                href={buyExplorerTxUrl(bs.onChainTx, bs.network)}
                target="_blank"
                rel="noreferrer"
                className="mt-1 inline-block text-xs text-primary-500 underline">
                View payment on Solana Explorer
              </a>
            )}
          </div>
        )}
        {buying && bs.phase === 'error' && (
          <div
            className="mt-3 rounded-md border border-red-500/30 bg-red-500/10 p-3"
            data-testid="buy-identity-error">
            <p className="text-xs font-medium text-red-500">
              {bs.onChainTx ? 'Payment sent but purchase did not complete.' : 'Purchase failed.'}
            </p>
            <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400">{bs.message}</p>
          </div>
        )}

        {/* Confirm-before-spend dialog. */}
        {buying && (bs.phase === 'confirm' || bs.phase === 'paying') && (
          <X402ConfirmDialog
            title={`Buy ${buying.name}`}
            subtitle="Confirm the x402 payment to buy this handle."
            amount={bs.challenge.amount ?? '0'}
            asset={bs.challenge.asset ?? 'USDC'}
            network={bs.challenge.network}
            balance={bs.balance}
            walletAddress={bs.walletAddress}
            busy={bs.phase === 'paying'}
            busyLabel="Paying…"
            onCancel={closeBuy}
            onConfirm={() =>
              buy.confirmPay(buying.listingId, bs.challenge, bs.balance, bs.walletAddress)
            }
          />
        )}
      </div>

      {/* Recent sales */}
      <div>
        <h3 className="mb-2 text-xs font-semibold uppercase tracking-wider text-stone-400 dark:text-neutral-500">
          Recent Sales
        </h3>
        <div className="overflow-hidden rounded-lg border border-stone-200 dark:border-neutral-800">
          {salesState.status === 'loading' && (
            <p className="p-3 text-xs text-stone-500 dark:text-neutral-400 animate-pulse">
              Loading sales…
            </p>
          )}
          {salesState.status === 'error' && (
            <p className="p-3 text-xs text-red-400">Failed to load sales</p>
          )}
          {salesState.status === 'ok' && sales.length === 0 && (
            <p className="p-3 text-xs text-stone-500 dark:text-neutral-400">No recent sales</p>
          )}
          {sales.length > 0 && (
            <table className="w-full text-left text-xs">
              <thead>
                <tr className="border-b border-stone-200 dark:border-neutral-800">
                  <th className="px-3 py-2 font-medium text-stone-400 dark:text-neutral-500">
                    Handle
                  </th>
                  <th className="px-3 py-2 font-medium text-stone-400 dark:text-neutral-500">
                    Price
                  </th>
                  <th className="px-3 py-2 font-medium text-stone-400 dark:text-neutral-500">
                    Buyer
                  </th>
                  <th className="px-3 py-2 text-right font-medium text-stone-400 dark:text-neutral-500">
                    Date
                  </th>
                </tr>
              </thead>
              <tbody>
                {sales.map((sale, index) => (
                  <tr
                    key={sale.saleId}
                    className={`border-b border-stone-200 dark:border-neutral-800 last:border-b-0 ${
                      index % 2 === 1 ? 'bg-stone-50 dark:bg-neutral-900/50' : ''
                    }`}>
                    <td className="px-3 py-2 font-medium text-stone-900 dark:text-neutral-100">
                      {sale.name}
                    </td>
                    <td className="px-3 py-2 text-stone-900 dark:text-neutral-100">
                      {formatPrice(sale.price.amount, sale.price.asset)}
                    </td>
                    <td className="px-3 py-2 font-mono text-stone-400 dark:text-neutral-500">
                      {sale.buyer.slice(0, 12)}...
                    </td>
                    <td className="px-3 py-2 text-right text-stone-400 dark:text-neutral-500">
                      {sale.createdAt.slice(0, 10)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      </div>
    </div>
  );
}

// ── Root section ──────────────────────────────────────────────────────────────

const TAB_KEYS: Record<Tab, string> = {
  register: 'Register',
  registry: 'Registry',
  trading: 'Trading',
};

// Tracks render count to force remount when switching tabs (clears local state)
type TabState = { tab: Tab; key: number };
type TabAction = { type: 'set'; tab: Tab };
function tabReducer(state: TabState, action: TabAction): TabState {
  if (action.tab === state.tab) return state;
  return { tab: action.tab, key: state.key + 1 };
}

export default function IdentitiesSection() {
  const [{ tab, key }, dispatch] = useReducer(tabReducer, { tab: 'register', key: 0 });
  // Incremented on successful registration. RegistryTab uses this as its React
  // `key` so it fully remounts and re-fetches the directory — making the newly
  // published agent card visible without a manual reload.
  const [registryKey, setRegistryKey] = useState(0);
  const bumpRegistryKey = useCallback(() => setRegistryKey(k => k + 1), []);

  return (
    <PanelScaffold description="Claim handles, manage your registry, and trade identities">
      <div className="flex gap-1">
        {(Object.keys(TAB_KEYS) as Tab[]).map(tabKey => (
          <button
            key={tabKey}
            type="button"
            onClick={() => {
              dispatch({ type: 'set', tab: tabKey });
            }}
            data-active={tab === tabKey}
            className={[
              'rounded-full px-3 py-1 text-xs font-medium transition-colors',
              tab === tabKey
                ? 'bg-stone-800 text-white dark:bg-neutral-100 dark:text-neutral-900'
                : 'border border-stone-200 bg-white text-stone-600 hover:bg-stone-50 dark:border-neutral-800 dark:bg-neutral-900 dark:text-neutral-300 dark:hover:bg-neutral-800',
            ].join(' ')}>
            {TAB_KEYS[tabKey]}
          </button>
        ))}
      </div>

      <div key={key}>
        {tab === 'register' && <RegisterTab onRegistered={bumpRegistryKey} />}
        {tab === 'registry' && <RegistryTab key={registryKey} />}
        {tab === 'trading' && <TradingTab />}
      </div>
    </PanelScaffold>
  );
}
