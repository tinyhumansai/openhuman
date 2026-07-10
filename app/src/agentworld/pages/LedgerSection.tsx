/**
 * LedgerSection — Agent World "Ledger" section.
 *
 * Renders the public transaction ledger via
 * `apiClient.graphql.ledgerTransactions()` (GraphQL, no auth required).
 * Supports inline row expansion to show full transaction details, metadata,
 * and a Solana explorer link via the shared `explorerTxUrl` helper.
 *
 * Pattern mirrors FeedSection: useState + useEffect fetch, PanelScaffold
 * wrapper, StatusBlock for loading/error/empty states.
 */
import { useEffect, useState } from 'react';

import PanelScaffold from '../../components/layout/PanelScaffold';
import { type GqlLedgerTransaction } from '../../lib/agentworld/invokeApiClient';
import { apiClient } from '../AgentWorldShell';
import { decimalsForAsset, resolveAssetSymbol } from '../assets';
import { formatUnits, friendlyNetwork } from '../components/X402ConfirmDialog';
import { explorerTxUrl } from '../hooks/useX402Buy';

// ── State types ───────────────────────────────────────────────────────────────

type LedgerState =
  | { status: 'loading' }
  | { status: 'error'; message: string }
  | { status: 'ok'; transactions: GqlLedgerTransaction[] };

// ── Helpers ───────────────────────────────────────────────────────────────────

function relativeTime(iso: string): string {
  const ms = Date.now() - new Date(iso).getTime();
  const mins = Math.floor(ms / 60000);
  if (mins < 1) return 'just now';
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  const days = Math.floor(hrs / 24);
  return `${days}d ago`;
}

export function abbreviateAddress(addr: string | undefined): string {
  if (!addr) return '—';
  if (addr.length <= 12) return addr;
  return `${addr.slice(0, 4)}…${addr.slice(-4)}`;
}

/**
 * Group the integer part of a numeric amount with thousands separators while
 * preserving the original decimal places (so "0.50" stays "0.50" and
 * "1000000" becomes "1,000,000"). Non-numeric strings pass through unchanged.
 */
export function formatAmount(amount: string | undefined): string {
  if (!amount) return '—';
  if (!Number.isFinite(Number(amount))) return amount;
  const negative = amount.startsWith('-');
  const body = negative ? amount.slice(1) : amount;
  const [intPart, fracPart] = body.split('.');
  const grouped = Number(intPart).toLocaleString('en-US');
  const out = fracPart != null ? `${grouped}.${fracPart}` : grouped;
  return negative ? `-${out}` : out;
}

/**
 * Ledger amounts arrive in the asset's smallest base unit (e.g. USDC in 1e-6
 * micro-units), so they must be scaled to display units before grouping —
 * otherwise every value reads ~1,000,000× too large. `asset` may be a symbol or
 * a mint address; {@link decimalsForAsset} resolves either.
 */
export function formatLedgerAmount(amount: string | undefined, asset: string | undefined): string {
  if (!amount) return formatAmount(amount);
  // `formatUnits` assumes an integer base-unit string; if the amount is already
  // decimal/non-integer, pass it straight to grouping instead of mis-scaling it.
  if (!/^-?\d+$/.test(amount)) return formatAmount(amount);
  const decimals = decimalsForAsset(asset);
  const display = decimals > 0 ? formatUnits(amount, decimals) : amount;
  return formatAmount(display);
}

/** Centered status message for loading / error / info states. */
function StatusBlock({ tone, title, body }: { tone: string; title: string; body?: string }) {
  return (
    <div className="flex h-64 flex-col items-center justify-center gap-2 text-center">
      <p className={`text-base font-medium ${tone}`}>{title}</p>
      {body && <p className="max-w-md text-sm text-content-muted">{body}</p>}
    </div>
  );
}

// ── StatusBadge ───────────────────────────────────────────────────────────────

export function StatusBadge({ status }: { status: string }) {
  const color =
    status === 'SETTLED'
      ? 'bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400'
      : status === 'PENDING'
        ? 'bg-amber-100 text-amber-700 dark:bg-amber-900/30 dark:text-amber-400'
        : status === 'FAILED'
          ? 'bg-red-100 text-red-700 dark:bg-red-900/30 dark:text-red-400'
          : 'bg-surface-subtle text-content-secondary';
  return (
    <span className={`inline-flex rounded-full px-2 py-0.5 text-xs font-medium ${color}`}>
      {status}
    </span>
  );
}

// ── TypeBadge ─────────────────────────────────────────────────────────────────

function TypeBadge({ type }: { type: string }) {
  const color =
    type === 'REGISTRATION'
      ? 'bg-primary-100 text-primary-700 dark:bg-primary-900/30 dark:text-primary-400'
      : type === 'SALE'
        ? 'bg-purple-100 text-purple-700 dark:bg-purple-900/30 dark:text-purple-400'
        : type === 'FEE'
          ? 'bg-surface-subtle text-content-secondary'
          : 'bg-surface-subtle text-content-secondary';
  return (
    <span
      className={`inline-flex rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide ${color}`}>
      {type}
    </span>
  );
}

// ── TypeIcon (leading circular glyph, colored by type) ──────────────────────────

function TypeIcon({ type }: { type: string }) {
  const color =
    type === 'REGISTRATION'
      ? 'bg-primary-50 text-primary-600 dark:bg-primary-900/30 dark:text-primary-400'
      : type === 'SALE'
        ? 'bg-purple-50 text-purple-600 dark:bg-purple-900/30 dark:text-purple-400'
        : 'bg-surface-subtle text-content-muted';
  return (
    <div
      className={`flex h-9 w-9 shrink-0 items-center justify-center rounded-full ${color}`}
      aria-hidden="true">
      <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M7 16V4m0 0L3 8m4-4l4 4m6 0v12m0 0l4-4m-4 4l-4-4"
        />
      </svg>
    </div>
  );
}

// ── TransactionRow ─────────────────────────────────────────────────────────────

function TransactionRow({
  tx,
  expanded,
  onToggle,
}: {
  tx: GqlLedgerTransaction;
  expanded: boolean;
  onToggle: () => void;
}) {
  return (
    <div className="border-b border-line-subtle last:border-0">
      {/* Summary row — leading icon · stacked content · fixed meta column */}
      <button
        type="button"
        onClick={onToggle}
        className="flex w-full items-start gap-3 px-4 py-3 text-left transition-colors hover:bg-surface-muted dark:hover:bg-surface-muted/50">
        <TypeIcon type={tx.type} />

        {/* Content */}
        <div className="min-w-0 flex-1">
          {/* Line 1: amount + type + status */}
          <div className="flex items-center gap-2">
            <span className="text-sm font-semibold text-content">
              {formatLedgerAmount(tx.amount, tx.asset)}
              {tx.asset ? ` ${resolveAssetSymbol(tx.asset)}` : ''}
            </span>
            <TypeBadge type={tx.type} />
            <StatusBadge status={tx.status} />
          </div>

          {/* Line 2: from → to · network */}
          <div className="mt-1 flex min-w-0 items-center gap-1.5 text-xs text-content-muted">
            <span className="font-mono">{abbreviateAddress(tx.from)}</span>
            <svg
              className="h-3 w-3 shrink-0 text-content-faint"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M13 7l5 5m0 0l-5 5m5-5H6"
              />
            </svg>
            <span className="font-mono">{abbreviateAddress(tx.to)}</span>
            <span className="text-content-faint dark:text-neutral-600">·</span>
            <span className="truncate">{friendlyNetwork(tx.network)}</span>
          </div>
        </div>

        {/* Fixed meta column: time + (view-on-chain + chevron) */}
        <div className="flex shrink-0 flex-col items-end gap-1.5">
          <span className="whitespace-nowrap text-xs text-content-faint">
            {relativeTime(tx.timestamp)}
          </span>
          <div className="flex items-center gap-2">
            {tx.onChainTx && (
              <a
                href={explorerTxUrl(tx.onChainTx, tx.network)}
                target="_blank"
                rel="noopener noreferrer"
                className="whitespace-nowrap text-xs font-medium text-primary-600 hover:text-primary-700 dark:text-primary-400 dark:hover:text-primary-300"
                onClick={e => e.stopPropagation()}>
                View on chain
              </a>
            )}
            <svg
              className={`h-4 w-4 shrink-0 text-content-faint transition-transform ${expanded ? 'rotate-180' : ''}`}
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M19 9l-7 7-7-7"
              />
            </svg>
          </div>
        </div>
      </button>

      {/* Expanded detail */}
      {expanded && (
        <div className="border-t border-line-subtle bg-surface-muted px-4 py-3">
          <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-xs">
            {/* Ledger TX ID */}
            <dt className="font-medium text-content-muted">Tx ID</dt>
            <dd className="break-all font-mono text-content">{tx.txId}</dd>

            {/* Visibility */}
            <dt className="font-medium text-content-muted">Visibility</dt>
            <dd className="text-content">{tx.visibility}</dd>

            {/* Full From */}
            <dt className="font-medium text-content-muted">From</dt>
            <dd className="break-all font-mono text-content">{tx.from ?? '-'}</dd>

            {/* Full To */}
            <dt className="font-medium text-content-muted">To</dt>
            <dd className="break-all font-mono text-content">{tx.to ?? '-'}</dd>

            {/* Reference */}
            {tx.reference && (
              <>
                <dt className="font-medium text-content-muted">Ref kind</dt>
                <dd className="text-content">{tx.reference.kind}</dd>

                {tx.reference.id && (
                  <>
                    <dt className="font-medium text-content-muted">Ref ID</dt>
                    <dd className="break-all font-mono text-content">{tx.reference.id}</dd>
                  </>
                )}
                {tx.reference.parentTxId && (
                  <>
                    <dt className="font-medium text-content-muted">Parent Tx</dt>
                    <dd className="break-all font-mono text-content">{tx.reference.parentTxId}</dd>
                  </>
                )}
                {tx.reference.rate && (
                  <>
                    <dt className="font-medium text-content-muted">Rate</dt>
                    <dd className="text-content">{tx.reference.rate}</dd>
                  </>
                )}
              </>
            )}
          </dl>

          {/* Metadata key-value table */}
          {tx.metadata && Object.keys(tx.metadata).length > 0 && (
            <div className="mt-2">
              <p className="mb-1 text-xs font-medium text-content-muted">Metadata</p>
              <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-xs">
                {Object.entries(tx.metadata).map(([key, val]) => (
                  <>
                    <dt key={`k-${key}`} className="font-medium text-content-muted">
                      {key}
                    </dt>
                    <dd key={`v-${key}`} className="break-all text-content">
                      {typeof val === 'string' ? val : JSON.stringify(val)}
                    </dd>
                  </>
                ))}
              </dl>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

// ── LedgerSection (main export) ───────────────────────────────────────────────

export default function LedgerSection() {
  const [ledgerState, setLedgerState] = useState<LedgerState>({ status: 'loading' });
  const [expandedTxId, setExpandedTxId] = useState<string | null>(null);

  // ── Fetch ledger transactions ──────────────────────────────────────────────
  useEffect(() => {
    let cancelled = false;
    setLedgerState({ status: 'loading' });

    // TODO(phase-2-follow-up): implement pagination with offset or cursor.
    void apiClient.graphql
      .ledgerTransactions({ limit: 50 })
      .then(result => {
        if (cancelled) return;
        const transactions = Array.isArray(result?.transactions) ? result.transactions : [];
        setLedgerState({ status: 'ok', transactions });
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        setLedgerState({ status: 'error', message: String(err) });
      });

    return () => {
      cancelled = true;
    };
  }, []);

  // ── Render ─────────────────────────────────────────────────────────────────

  let body: React.ReactNode;

  if (ledgerState.status === 'loading') {
    body = (
      <div className="flex h-64 items-center justify-center text-content-faint">
        <span className="animate-pulse text-sm">Loading ledger…</span>
      </div>
    );
  } else if (ledgerState.status === 'error') {
    body = (
      <StatusBlock
        tone="text-red-600 dark:text-red-400"
        title="Failed to load ledger"
        body={ledgerState.message}
      />
    );
  } else if (ledgerState.transactions.length === 0) {
    body = (
      <StatusBlock
        tone="text-content-muted"
        title="No transactions found"
        body="The ledger is empty or no transactions match the current filter."
      />
    );
  } else {
    body = (
      <div className="rounded-lg border border-line bg-surface">
        {ledgerState.transactions.map(tx => (
          <TransactionRow
            key={tx.txId}
            tx={tx}
            expanded={expandedTxId === tx.txId}
            onToggle={() => setExpandedTxId(prev => (prev === tx.txId ? null : tx.txId))}
          />
        ))}
      </div>
    );
  }

  return <PanelScaffold description="Ledger">{body}</PanelScaffold>;
}
