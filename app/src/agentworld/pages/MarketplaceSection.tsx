/**
 * MarketplaceSection — Agent World Marketplace section.
 *
 * Renders a sub-tab bar (Search / Jobs / Post / Active / Delivered / Disputes /
 * Artifacts) and mounts the active tab component. Each tab calls into the
 * invoke API client bridge (`openhuman.tinyplace_graphql_*`,
 * `openhuman.tinyplace_marketplace_*`,
 * `openhuman.tinyplace_artifacts_*`, `openhuman.tinyplace_escrow_*`,
 * `openhuman.tinyplace_jobs_*`) via `apiClient` from `AgentWorldShell`.
 *
 * Design: follows the ExploreSection template — no vendor components, no dynamic
 * imports, no third-party state libraries. All data is fetched with local React
 * state + `useEffect`.
 */
import { useEffect, useState } from 'react';

import ChipTabs from '../../components/layout/ChipTabs';
import PanelScaffold from '../../components/layout/PanelScaffold';
import Button from '../../components/ui/Button';
import {
  type ArtifactListResult,
  type EscrowListResponse,
  type GqlJobListResult,
  PaymentRequiredError,
  type Product,
  type ProductsResponse,
} from '../../lib/agentworld/invokeApiClient';
import { apiClient } from '../AgentWorldShell';
import X402ConfirmDialog from '../components/X402ConfirmDialog';
import { explorerTxUrl, useX402Buy } from '../hooks/useX402Buy';

// ── Tab definitions ───────────────────────────────────────────────────────────

const TABS = ['search', 'jobs', 'active', 'delivered', 'artifacts'] as const;
type Tab = (typeof TABS)[number];

const TAB_LABELS: Record<Tab, string> = {
  search: 'Search',
  jobs: 'Jobs',
  active: 'Active',
  delivered: 'Delivered',
  artifacts: 'Artifacts',
};

// ── Shared state types ────────────────────────────────────────────────────────

type AsyncState<T> =
  | { status: 'loading' }
  | { status: 'payment_required'; challenge: unknown }
  | { status: 'error'; message: string }
  | { status: 'ok'; data: T };

// ── Shared helpers ────────────────────────────────────────────────────────────

function handleError<T>(
  err: unknown,
  setState: React.Dispatch<React.SetStateAction<AsyncState<T>>>
): void {
  if (err instanceof PaymentRequiredError) {
    setState({ status: 'payment_required', challenge: err.challenge });
  } else {
    setState({ status: 'error', message: String(err) });
  }
}

// ── Sub-tab: Search (browse + list products) ──────────────────────────────────

function SearchTab() {
  const [state, setState] = useState<AsyncState<ProductsResponse>>({ status: 'loading' });
  const [query, setQuery] = useState('');

  useEffect(() => {
    let cancelled = false;

    void apiClient.graphql
      .products()
      .then(data => {
        if (!cancelled) setState({ status: 'ok', data });
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        handleError(err, setState);
      });

    return () => {
      cancelled = true;
    };
  }, []);

  // x402 buy flow — the selected product drives the confirm dialog.
  const [buying, setBuying] = useState<Product | null>(null);
  const buy = useX402Buy((id, opts) => apiClient.marketplace.buyProduct(id, opts));

  function startBuy(product: Product) {
    setBuying(product);
    buy.begin(product.productId);
  }
  function closeBuy() {
    buy.reset();
    setBuying(null);
  }

  if (state.status === 'loading') {
    return <LoadingSpinner label="Loading products…" />;
  }
  if (state.status === 'payment_required') {
    return <PaymentRequired />;
  }
  if (state.status === 'error') {
    return <ErrorState message={state.message} />;
  }

  const bs = buy.state;
  const products = state.data.products ?? [];
  const normalizedQuery = query.trim().toLowerCase();
  const filtered = normalizedQuery
    ? products.filter(p => {
        const haystack = [p.name, p.description, p.seller, ...(p.tags ?? [])]
          .join(' ')
          .toLowerCase();
        return haystack.includes(normalizedQuery);
      })
    : products;

  return (
    <div className="flex flex-col gap-4">
      <input
        className="w-full rounded-lg border border-line-strong bg-surface px-3 py-2 text-sm text-content placeholder-content-faint focus:border-primary-500 focus:outline-none"
        placeholder="Search products by name, description, tag, or seller…"
        type="search"
        value={query}
        onChange={e => setQuery(e.target.value)}
      />

      {products.length === 0 ? (
        <EmptyState label="No products listed yet." />
      ) : filtered.length === 0 ? (
        <EmptyState label="No products match your search." />
      ) : (
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
          {filtered.map(product => (
            <div key={product.productId} className="rounded-xl border border-line bg-surface p-4">
              <div className="flex items-start justify-between gap-2">
                <span className="text-sm font-medium text-content">{product.name}</span>
                <span className="shrink-0 rounded-full bg-surface-subtle px-2 py-0.5 text-xs text-content-muted">
                  {product.category}
                </span>
              </div>
              <p className="mt-1 text-xs text-content-faint">{product.description}</p>
              <div className="mt-2 flex items-center justify-between">
                <span className="text-xs text-content-muted">{product.seller}</span>
                <span className="text-xs font-medium text-content">
                  {product.price.amount} {product.price.asset}
                </span>
              </div>
              {product.tags && product.tags.length > 0 && (
                <div className="mt-2 flex flex-wrap gap-1">
                  {product.tags.map(tag => (
                    <span
                      key={tag}
                      className="rounded-full bg-surface-subtle px-1.5 py-0.5 text-[10px] text-content-faint">
                      {tag}
                    </span>
                  ))}
                </div>
              )}
              <Button
                variant="primary"
                size="sm"
                disabled={buying !== null}
                onClick={() => startBuy(product)}
                className="mt-3 w-full">
                Buy
              </Button>
            </div>
          ))}
        </div>
      )}

      {/* Buy outcome banner (success / error survives the dialog closing). */}
      {buying && bs.phase === 'success' && (
        <div
          className="rounded-md border border-green-500/30 bg-green-500/10 p-3"
          data-testid="buy-success">
          <p className="text-xs font-medium text-green-500">Purchased {buying.name}</p>
          {bs.onChainTx && (
            <a
              href={explorerTxUrl(bs.onChainTx, bs.network)}
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
          className="rounded-md border border-red-500/30 bg-red-500/10 p-3"
          data-testid="buy-error">
          <p className="text-xs font-medium text-red-500">
            {bs.onChainTx ? 'Payment sent but purchase did not complete.' : 'Purchase failed.'}
          </p>
          <p className="mt-1 text-xs text-content-muted">{bs.message}</p>
          {bs.onChainTx && (
            <a
              href={explorerTxUrl(bs.onChainTx)}
              target="_blank"
              rel="noreferrer"
              className="mt-1 inline-block text-xs text-primary-500 underline">
              View payment on Solana Explorer
            </a>
          )}
        </div>
      )}

      {/* Confirm-before-spend dialog. */}
      {buying && (bs.phase === 'confirm' || bs.phase === 'paying') && (
        <X402ConfirmDialog
          title={`Buy ${buying.name}`}
          subtitle="Confirm the x402 payment to complete this purchase."
          amount={bs.challenge.amount ?? '0'}
          asset={bs.challenge.asset ?? 'USDC'}
          network={bs.challenge.network}
          balance={bs.balance}
          walletAddress={bs.walletAddress}
          busy={bs.phase === 'paying'}
          busyLabel="Paying…"
          onCancel={closeBuy}
          onConfirm={() =>
            buy.confirmPay(buying.productId, bs.challenge, bs.balance, bs.walletAddress)
          }
        />
      )}
    </div>
  );
}

// ── Sub-tab: Jobs ─────────────────────────────────────────────────────────────

function JobsTab() {
  const [state, setState] = useState<AsyncState<GqlJobListResult>>({ status: 'loading' });

  useEffect(() => {
    let cancelled = false;

    void apiClient.graphql
      .jobs({ limit: 50 })
      .then(data => {
        if (!cancelled) setState({ status: 'ok', data });
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        handleError(err, setState);
      });

    return () => {
      cancelled = true;
    };
  }, []);

  if (state.status === 'loading') {
    return <LoadingSpinner label="Loading jobs…" />;
  }
  if (state.status === 'payment_required') {
    return <PaymentRequired />;
  }
  if (state.status === 'error') {
    return <ErrorState message={state.message} />;
  }

  const jobs = state.data.jobs ?? [];

  if (jobs.length === 0) {
    return <EmptyState label="No job postings yet." />;
  }

  return (
    <div className="flex flex-col gap-3">
      {jobs.map(job => (
        <div key={job.jobId} className="rounded-xl border border-line bg-surface p-4">
          <div className="flex items-center justify-between gap-2">
            <span className="text-sm font-medium text-content">
              {typeof job.title === 'string' ? job.title : job.jobId}
            </span>
            <StatusBadge status={job.status} />
          </div>
          {typeof job.description === 'string' && (
            <p className="mt-1 text-xs text-content-faint">{job.description}</p>
          )}
          <span className="mt-2 block text-xs text-content-muted">{job.client}</span>
        </div>
      ))}
    </div>
  );
}

// ── Sub-tab: Active (escrows in-progress) ─────────────────────────────────────

const ACTIVE_STATUSES = new Set(['funded', 'accepted', 'revision_requested']);

function ActiveTab() {
  const [state, setState] = useState<AsyncState<EscrowListResponse>>({ status: 'loading' });

  useEffect(() => {
    let cancelled = false;

    void apiClient.escrow
      .list()
      .then(data => {
        if (!cancelled) setState({ status: 'ok', data });
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        handleError(err, setState);
      });

    return () => {
      cancelled = true;
    };
  }, []);

  if (state.status === 'loading') {
    return <LoadingSpinner label="Loading active work…" />;
  }
  if (state.status === 'payment_required') {
    return <PaymentRequired />;
  }
  if (state.status === 'error') {
    return <ErrorState message={state.message} />;
  }

  const active = (state.data.escrows ?? []).filter(e => ACTIVE_STATUSES.has(e.status));

  if (active.length === 0) {
    return <EmptyState label="No work in progress. Hire or accept a job to get started." />;
  }

  return (
    <div className="flex flex-col gap-3">
      {active.map(escrow => (
        <EscrowRow key={escrow.escrowId} escrow={escrow} />
      ))}
    </div>
  );
}

// ── Sub-tab: Delivered (completed / settled escrows) ──────────────────────────

const DELIVERED_STATUSES = new Set(['delivered', 'settled', 'resolved', 'cancelled', 'expired']);

function DeliveredTab() {
  const [state, setState] = useState<AsyncState<EscrowListResponse>>({ status: 'loading' });

  useEffect(() => {
    let cancelled = false;

    void apiClient.escrow
      .list()
      .then(data => {
        if (!cancelled) setState({ status: 'ok', data });
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        handleError(err, setState);
      });

    return () => {
      cancelled = true;
    };
  }, []);

  if (state.status === 'loading') {
    return <LoadingSpinner label="Loading delivered work…" />;
  }
  if (state.status === 'payment_required') {
    return <PaymentRequired />;
  }
  if (state.status === 'error') {
    return <ErrorState message={state.message} />;
  }

  const delivered = (state.data.escrows ?? []).filter(e => DELIVERED_STATUSES.has(e.status));

  if (delivered.length === 0) {
    return <EmptyState label="Nothing delivered yet." />;
  }

  return (
    <div className="flex flex-col gap-3">
      {delivered.map(escrow => (
        <EscrowRow key={escrow.escrowId} escrow={escrow} />
      ))}
    </div>
  );
}

// ── Sub-tab: Artifacts ────────────────────────────────────────────────────────

function ArtifactsTab() {
  const [state, setState] = useState<AsyncState<ArtifactListResult>>({ status: 'loading' });

  useEffect(() => {
    let cancelled = false;

    void apiClient.artifacts
      .list()
      .then(data => {
        if (!cancelled) setState({ status: 'ok', data });
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        handleError(err, setState);
      });

    return () => {
      cancelled = true;
    };
  }, []);

  if (state.status === 'loading') {
    return <LoadingSpinner label="Loading artifacts…" />;
  }
  if (state.status === 'payment_required') {
    return <PaymentRequired />;
  }
  if (state.status === 'error') {
    return <ErrorState message={state.message} />;
  }

  const artifacts = state.data.artifacts ?? [];

  if (artifacts.length === 0) {
    return <EmptyState label="No artifacts yet." />;
  }

  return (
    <div className="flex flex-col gap-3">
      {artifacts.map(artifact => (
        <div key={artifact.artifactId} className="rounded-xl border border-line bg-surface p-4">
          <div className="flex items-center justify-between gap-2">
            <span className="text-sm font-medium text-content">
              {artifact.name ?? artifact.artifactId}
            </span>
            {artifact.status && <StatusBadge status={artifact.status} />}
          </div>
          {artifact.description && (
            <p className="mt-1 text-xs text-content-faint">{artifact.description}</p>
          )}
          <div className="mt-2 flex items-center gap-3 text-xs text-content-muted">
            <span>{artifact.mimeType ?? 'unknown type'}</span>
            {artifact.sizeBytes !== undefined && (
              <span>{(artifact.sizeBytes / 1024).toFixed(1)} KB</span>
            )}
          </div>
        </div>
      ))}
    </div>
  );
}

// ── Shared small components ───────────────────────────────────────────────────

function LoadingSpinner({ label }: { label: string }) {
  return (
    <div className="flex items-center justify-center py-12">
      <span className="animate-pulse text-sm text-content-muted">{label}</span>
    </div>
  );
}

function PaymentRequired() {
  return (
    <div className="flex flex-col items-center justify-center gap-4 py-12 text-amber-400">
      <p className="text-lg font-medium">Access requires payment</p>
      <p className="text-sm text-content-muted">
        Your wallet will be used to fulfill the x402 payment challenge.
      </p>
    </div>
  );
}

function ErrorState({ message }: { message: string }) {
  const isWalletLocked =
    message.includes('wallet is not configured') ||
    message.includes('wallet secret material is missing');

  if (isWalletLocked) {
    return (
      <div className="flex flex-col items-center justify-center gap-4 py-12 text-content-muted">
        <p className="text-lg font-medium">Unlock your wallet to use Agent World</p>
        <p className="text-sm">
          Agent World uses your wallet identity. Import your recovery phrase in Settings to
          continue.
        </p>
      </div>
    );
  }

  return (
    <div className="flex flex-col items-center justify-center gap-2 py-12 text-red-400">
      <p className="font-medium">Failed to load</p>
      <p className="text-sm text-content-faint">{message}</p>
    </div>
  );
}

function EmptyState({ label }: { label: string }) {
  return (
    <div className="flex items-center justify-center py-12">
      <span className="text-sm text-content-muted">{label}</span>
    </div>
  );
}

function StatusBadge({ status }: { status: string }) {
  const colorMap: Record<string, string> = {
    active: 'bg-green-100 text-green-700 dark:bg-green-900/40 dark:text-green-300',
    funded: 'bg-blue-100 text-blue-700 dark:bg-blue-900/40 dark:text-blue-300',
    accepted: 'bg-primary-100 text-primary-700 dark:bg-primary-900/40 dark:text-primary-300',
    delivered: 'bg-purple-100 text-purple-700 dark:bg-purple-900/40 dark:text-purple-300',
    settled: 'bg-surface-strong text-content-secondary',
    cancelled: 'bg-red-100 text-red-700 dark:bg-red-900/40 dark:text-red-300',
    expired: 'bg-surface-subtle text-content-faint',
  };
  const cls = colorMap[status] ?? 'bg-surface-subtle text-content-muted';
  return <span className={`rounded-full px-2 py-0.5 text-xs font-medium ${cls}`}>{status}</span>;
}

function EscrowRow({
  escrow,
}: {
  escrow: {
    escrowId: string;
    status: string;
    client: string;
    provider: string;
    [key: string]: unknown;
  };
}) {
  return (
    <div className="rounded-xl border border-line bg-surface p-4">
      <div className="flex items-center justify-between gap-2">
        <span className="text-sm font-medium text-content">
          {typeof escrow.title === 'string' ? escrow.title : escrow.escrowId}
        </span>
        <StatusBadge status={escrow.status} />
      </div>
      <div className="mt-2 flex items-center gap-3 text-xs text-content-muted">
        <span>Client: {escrow.client}</span>
        <span>Provider: {escrow.provider}</span>
      </div>
    </div>
  );
}

// ── Tab component map ─────────────────────────────────────────────────────────

const TAB_COMPONENTS: Record<Tab, React.ComponentType> = {
  search: SearchTab,
  jobs: JobsTab,
  active: ActiveTab,
  delivered: DeliveredTab,
  artifacts: ArtifactsTab,
};

// ── MarketplaceSection ────────────────────────────────────────────────────────

export default function MarketplaceSection() {
  const [activeTab, setActiveTab] = useState<Tab>('search');
  const ActiveComponent = TAB_COMPONENTS[activeTab];

  return (
    <PanelScaffold description="Browse products, jobs, escrows, and artifacts">
      {/* Sub-tab chips — canonical ChipTabs (Settings → Account bubble look) */}
      <ChipTabs
        as="tab"
        ariaLabel="Marketplace sections"
        className="flex flex-wrap gap-1.5"
        items={TABS.map(tab => ({ id: tab, label: TAB_LABELS[tab] }))}
        value={activeTab}
        onChange={setActiveTab}
      />

      <ActiveComponent />
    </PanelScaffold>
  );
}
