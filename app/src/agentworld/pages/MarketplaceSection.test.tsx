/**
 * Tests for MarketplaceSection — the Agent World Marketplace section.
 *
 * Covers all five sub-tabs (Search / Jobs / Active / Delivered / Artifacts),
 * each AsyncState branch (loading / error / payment_required / empty /
 * populated), and the interactive behaviour (tab switches, search filter,
 * wallet-locked error styling, status badges, escrow filtering).
 *
 * The apiClient is mocked so no real RPC calls are made. All sample data uses
 * generic placeholder names/ids.
 */
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { PaymentRequiredError } from '../../lib/agentworld/invokeApiClient';
import { apiClient } from '../AgentWorldShell';
import MarketplaceSection from './MarketplaceSection';

// ── Mock apiClient ────────────────────────────────────────────────────────────
// The page imports `apiClient` as a named export from AgentWorldShell. We
// replace only the methods MarketplaceSection actually calls.

vi.mock('../AgentWorldShell', () => ({
  apiClient: {
    graphql: { jobs: vi.fn(), products: vi.fn() },
    marketplace: { buyProduct: vi.fn() },
    escrow: { list: vi.fn() },
    artifacts: { list: vi.fn() },
  },
}));

// ── Sample data (generic placeholders only) ───────────────────────────────────

const sampleProduct = {
  productId: 'prod-1',
  seller: 'seller-alpha',
  sellerCryptoId: 'crypto-1',
  name: 'Widget Builder',
  description: 'Builds widgets fast',
  category: 'tools',
  tags: ['fast', 'widgets'],
  price: { amount: '10', asset: 'USDC' },
  deliveryMethod: 'instant',
  status: 'active',
  createdAt: '2026-01-01T00:00:00Z',
  updatedAt: '2026-01-01T00:00:00Z',
  salesCount: 0,
  rating: 0,
};

const sampleProductNoTags = {
  ...sampleProduct,
  productId: 'prod-2',
  name: 'Gadget Maker',
  description: 'Makes gadgets',
  seller: 'seller-beta',
  tags: undefined,
};

const sampleJob = {
  jobId: 'job-1',
  status: 'funded',
  client: 'client-alpha',
  title: 'Translate document',
  description: 'Translate from EN to FR',
  budget: { amount: '50', asset: 'USDC' },
  proposalCount: 0,
  createdAt: '2026-01-01T00:00:00Z',
  updatedAt: '2026-01-01T00:00:00Z',
  clientProfile: {
    handle: 'client-alpha',
    cryptoId: 'client-alpha',
    displayName: 'Client Alpha',
    verified: false,
  },
};

const sampleJobNoTitle = {
  ...sampleJob,
  jobId: 'job-2',
  status: 'open',
  client: 'client-beta',
  title: undefined,
};

const activeEscrow = {
  escrowId: 'esc-active-1',
  status: 'funded',
  client: 'client-1',
  provider: 'provider-1',
  title: 'Active task',
};

const deliveredEscrow = {
  escrowId: 'esc-done-1',
  status: 'settled',
  client: 'client-2',
  provider: 'provider-2',
};

const sampleArtifact = {
  artifactId: 'art-1',
  owner: 'owner-1',
  name: 'report.pdf',
  description: 'Quarterly report',
  mimeType: 'application/pdf',
  sizeBytes: 2048,
  status: 'delivered',
};

const sampleArtifactMinimal = { artifactId: 'art-2', owner: 'owner-2' };

// ── Default mock state: every fetch resolves empty ────────────────────────────

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(apiClient.graphql.products).mockResolvedValue({ products: [] });
  vi.mocked(apiClient.graphql.jobs).mockResolvedValue({ jobs: [], count: 0 });
  vi.mocked(apiClient.escrow.list).mockResolvedValue({ escrows: [] });
  vi.mocked(apiClient.artifacts.list).mockResolvedValue({ artifacts: [] });
  vi.mocked(apiClient.marketplace.buyProduct).mockResolvedValue({ result: { purchaseId: 'p1' } });
});

// ── Tab navigation ────────────────────────────────────────────────────────────

describe('tab navigation', () => {
  test('defaults to the Search tab', () => {
    render(<MarketplaceSection />);
    const searchBtn = screen.getByRole('tab', { name: 'Search' });
    expect(searchBtn).toHaveAttribute('aria-selected', 'true');
  });

  test('renders all five sub-tabs', () => {
    render(<MarketplaceSection />);
    for (const label of ['Search', 'Jobs', 'Active', 'Delivered', 'Artifacts']) {
      expect(screen.getByRole('tab', { name: label })).toBeInTheDocument();
    }
  });

  test('switching to Jobs marks it selected and fetches jobs', async () => {
    render(<MarketplaceSection />);
    await userEvent.click(screen.getByRole('tab', { name: 'Jobs' }));
    expect(screen.getByRole('tab', { name: 'Jobs' })).toHaveAttribute('aria-selected', 'true');
    expect(apiClient.graphql.jobs).toHaveBeenCalledWith({ limit: 50 });
  });

  test('switching to Active fetches escrows', async () => {
    render(<MarketplaceSection />);
    await userEvent.click(screen.getByRole('tab', { name: 'Active' }));
    expect(apiClient.escrow.list).toHaveBeenCalled();
  });

  test('switching to Artifacts fetches artifacts', async () => {
    render(<MarketplaceSection />);
    await userEvent.click(screen.getByRole('tab', { name: 'Artifacts' }));
    expect(apiClient.artifacts.list).toHaveBeenCalled();
  });
});

// ── Search tab ────────────────────────────────────────────────────────────────

describe('Search tab', () => {
  test('shows the loading spinner before the fetch resolves', () => {
    // A never-resolving promise keeps the tab in its loading state.
    vi.mocked(apiClient.graphql.products).mockReturnValue(new Promise(() => {}));
    render(<MarketplaceSection />);
    expect(screen.getByText('Loading products…')).toBeInTheDocument();
  });

  test('shows empty state when no products exist', async () => {
    render(<MarketplaceSection />);
    expect(await screen.findByText('No products listed yet.')).toBeInTheDocument();
  });

  test('renders populated products with tags, price and category', async () => {
    vi.mocked(apiClient.graphql.products).mockResolvedValue({
      products: [sampleProduct, sampleProductNoTags],
    });
    render(<MarketplaceSection />);
    expect(await screen.findByText('Widget Builder')).toBeInTheDocument();
    expect(screen.getByText('Gadget Maker')).toBeInTheDocument();
    // Price renders as `{amount} {asset}` in a single span; both products
    // share a price so there are two category badges as well.
    expect(screen.getAllByText(/USDC/).length).toBeGreaterThan(0);
    expect(screen.getAllByText('tools').length).toBe(2); // category badges
    expect(screen.getByText('fast')).toBeInTheDocument();
    expect(screen.getByText('widgets')).toBeInTheDocument();
  });

  test('search input filters products by name', async () => {
    vi.mocked(apiClient.graphql.products).mockResolvedValue({
      products: [sampleProduct, sampleProductNoTags],
    });
    render(<MarketplaceSection />);
    await screen.findByText('Widget Builder');

    const input = screen.getByPlaceholderText(/Search products by name/i);
    await userEvent.type(input, 'gadget');

    expect(screen.queryByText('Widget Builder')).not.toBeInTheDocument();
    expect(screen.getByText('Gadget Maker')).toBeInTheDocument();
  });

  test('search input shows "no match" empty state when nothing matches', async () => {
    vi.mocked(apiClient.graphql.products).mockResolvedValue({ products: [sampleProduct] });
    render(<MarketplaceSection />);
    await screen.findByText('Widget Builder');

    const input = screen.getByPlaceholderText(/Search products by name/i);
    await userEvent.type(input, 'zzzznomatch');

    expect(await screen.findByText('No products match your search.')).toBeInTheDocument();
  });

  test('search matches against seller and tags too', async () => {
    vi.mocked(apiClient.graphql.products).mockResolvedValue({
      products: [sampleProduct, sampleProductNoTags],
    });
    render(<MarketplaceSection />);
    await screen.findByText('Widget Builder');

    const input = screen.getByPlaceholderText(/Search products by name/i);
    await userEvent.type(input, 'seller-beta');

    expect(screen.getByText('Gadget Maker')).toBeInTheDocument();
    expect(screen.queryByText('Widget Builder')).not.toBeInTheDocument();
  });

  test('tolerates a response missing the products field', async () => {
    vi.mocked(apiClient.graphql.products).mockResolvedValue({} as { products: never[] });
    render(<MarketplaceSection />);
    expect(await screen.findByText('No products listed yet.')).toBeInTheDocument();
  });

  test('shows the generic error state on a plain rejection', async () => {
    vi.mocked(apiClient.graphql.products).mockRejectedValueOnce(new Error('boom'));
    render(<MarketplaceSection />);
    expect(await screen.findByText('Failed to load')).toBeInTheDocument();
    expect(screen.getByText(/boom/)).toBeInTheDocument();
  });

  test('shows the wallet-locked error state when wallet is not configured', async () => {
    vi.mocked(apiClient.graphql.products).mockRejectedValueOnce(
      new Error('wallet is not configured')
    );
    render(<MarketplaceSection />);
    expect(await screen.findByText('Unlock your wallet to use Agent World')).toBeInTheDocument();
  });

  test('shows the wallet-locked error state when wallet secret material is missing', async () => {
    vi.mocked(apiClient.graphql.products).mockRejectedValueOnce(
      new Error('wallet secret material is missing')
    );
    render(<MarketplaceSection />);
    expect(await screen.findByText('Unlock your wallet to use Agent World')).toBeInTheDocument();
  });

  test('shows the payment-required state on a PaymentRequiredError', async () => {
    vi.mocked(apiClient.graphql.products).mockRejectedValueOnce(
      new PaymentRequiredError({ terms: 'x402' })
    );
    render(<MarketplaceSection />);
    expect(await screen.findByText('Access requires payment')).toBeInTheDocument();
  });
});

// ── Jobs tab ──────────────────────────────────────────────────────────────────

describe('Jobs tab', () => {
  async function openJobs() {
    render(<MarketplaceSection />);
    await userEvent.click(screen.getByRole('tab', { name: 'Jobs' }));
  }

  test('shows empty state when there are no jobs', async () => {
    await openJobs();
    expect(await screen.findByText('No job postings yet.')).toBeInTheDocument();
  });

  test('renders a job with title, description and status badge', async () => {
    vi.mocked(apiClient.graphql.jobs).mockResolvedValue({ jobs: [sampleJob] });
    await openJobs();
    expect(await screen.findByText('Translate document')).toBeInTheDocument();
    expect(screen.getByText('Translate from EN to FR')).toBeInTheDocument();
    expect(screen.getByText('funded')).toBeInTheDocument();
    expect(screen.getByText('client-alpha')).toBeInTheDocument();
  });

  test('falls back to jobId when title is not a string', async () => {
    vi.mocked(apiClient.graphql.jobs).mockResolvedValue({ jobs: [sampleJobNoTitle as never] });
    await openJobs();
    expect(await screen.findByText('job-2')).toBeInTheDocument();
    // Unknown status uses the default badge styling but still renders the label.
    expect(screen.getByText('open')).toBeInTheDocument();
  });

  test('tolerates a response missing the jobs field', async () => {
    vi.mocked(apiClient.graphql.jobs).mockResolvedValue({} as { jobs: never[] });
    await openJobs();
    expect(await screen.findByText('No job postings yet.')).toBeInTheDocument();
  });

  test('shows the error state on rejection', async () => {
    vi.mocked(apiClient.graphql.jobs).mockRejectedValueOnce(new Error('jobs down'));
    await openJobs();
    expect(await screen.findByText('Failed to load')).toBeInTheDocument();
  });

  test('shows payment-required on a PaymentRequiredError', async () => {
    vi.mocked(apiClient.graphql.jobs).mockRejectedValueOnce(new PaymentRequiredError(null));
    await openJobs();
    expect(await screen.findByText('Access requires payment')).toBeInTheDocument();
  });

  test('shows the loading spinner while jobs are pending', async () => {
    vi.mocked(apiClient.graphql.jobs).mockReturnValue(new Promise(() => {}));
    await openJobs();
    expect(screen.getByText('Loading jobs…')).toBeInTheDocument();
  });
});

// ── Active tab ────────────────────────────────────────────────────────────────

describe('Active tab', () => {
  async function openActive() {
    render(<MarketplaceSection />);
    await userEvent.click(screen.getByRole('tab', { name: 'Active' }));
  }

  test('shows empty state when there are no in-progress escrows', async () => {
    await openActive();
    expect(
      await screen.findByText('No work in progress. Hire or accept a job to get started.')
    ).toBeInTheDocument();
  });

  test('renders only escrows in active statuses', async () => {
    vi.mocked(apiClient.escrow.list).mockResolvedValue({
      escrows: [activeEscrow, deliveredEscrow],
    });
    await openActive();
    expect(await screen.findByText('Active task')).toBeInTheDocument();
    // The settled escrow must be filtered out of the Active tab.
    expect(screen.queryByText('esc-done-1')).not.toBeInTheDocument();
    expect(screen.getByText(/Client: client-1/)).toBeInTheDocument();
    expect(screen.getByText(/Provider: provider-1/)).toBeInTheDocument();
  });

  test('shows empty state when only delivered escrows exist', async () => {
    vi.mocked(apiClient.escrow.list).mockResolvedValue({ escrows: [deliveredEscrow] });
    await openActive();
    expect(
      await screen.findByText('No work in progress. Hire or accept a job to get started.')
    ).toBeInTheDocument();
  });

  test('shows the error state on rejection', async () => {
    vi.mocked(apiClient.escrow.list).mockRejectedValueOnce(new Error('escrow down'));
    await openActive();
    expect(await screen.findByText('Failed to load')).toBeInTheDocument();
  });

  test('shows payment-required on a PaymentRequiredError', async () => {
    vi.mocked(apiClient.escrow.list).mockRejectedValueOnce(new PaymentRequiredError(null));
    await openActive();
    expect(await screen.findByText('Access requires payment')).toBeInTheDocument();
  });

  test('shows the loading spinner while escrows are pending', async () => {
    vi.mocked(apiClient.escrow.list).mockReturnValue(new Promise(() => {}));
    await openActive();
    expect(screen.getByText('Loading active work…')).toBeInTheDocument();
  });

  test('tolerates a response missing the escrows field', async () => {
    vi.mocked(apiClient.escrow.list).mockResolvedValue({} as { escrows: never[] });
    await openActive();
    expect(
      await screen.findByText('No work in progress. Hire or accept a job to get started.')
    ).toBeInTheDocument();
  });
});

// ── Delivered tab ─────────────────────────────────────────────────────────────

describe('Delivered tab', () => {
  async function openDelivered() {
    render(<MarketplaceSection />);
    await userEvent.click(screen.getByRole('tab', { name: 'Delivered' }));
  }

  test('shows empty state when nothing is delivered', async () => {
    await openDelivered();
    expect(await screen.findByText('Nothing delivered yet.')).toBeInTheDocument();
  });

  test('renders only escrows in delivered statuses (falling back to id for missing title)', async () => {
    vi.mocked(apiClient.escrow.list).mockResolvedValue({
      escrows: [activeEscrow, deliveredEscrow],
    });
    await openDelivered();
    // deliveredEscrow has no title → falls back to its escrowId.
    expect(await screen.findByText('esc-done-1')).toBeInTheDocument();
    expect(screen.getByText('settled')).toBeInTheDocument();
    // The funded (active) escrow must not appear here.
    expect(screen.queryByText('Active task')).not.toBeInTheDocument();
  });

  test('shows the error state on rejection', async () => {
    vi.mocked(apiClient.escrow.list).mockRejectedValueOnce(new Error('escrow down'));
    await openDelivered();
    expect(await screen.findByText('Failed to load')).toBeInTheDocument();
  });

  test('shows payment-required on a PaymentRequiredError', async () => {
    vi.mocked(apiClient.escrow.list).mockRejectedValueOnce(new PaymentRequiredError(null));
    await openDelivered();
    expect(await screen.findByText('Access requires payment')).toBeInTheDocument();
  });

  test('shows the loading spinner while escrows are pending', async () => {
    vi.mocked(apiClient.escrow.list).mockReturnValue(new Promise(() => {}));
    await openDelivered();
    expect(screen.getByText('Loading delivered work…')).toBeInTheDocument();
  });
});

// ── Artifacts tab ─────────────────────────────────────────────────────────────

describe('Artifacts tab', () => {
  async function openArtifacts() {
    render(<MarketplaceSection />);
    await userEvent.click(screen.getByRole('tab', { name: 'Artifacts' }));
  }

  test('shows empty state when there are no artifacts', async () => {
    await openArtifacts();
    expect(await screen.findByText('No artifacts yet.')).toBeInTheDocument();
  });

  test('renders a fully-populated artifact with size, type, status and description', async () => {
    vi.mocked(apiClient.artifacts.list).mockResolvedValue({ artifacts: [sampleArtifact] });
    await openArtifacts();
    expect(await screen.findByText('report.pdf')).toBeInTheDocument();
    expect(screen.getByText('Quarterly report')).toBeInTheDocument();
    expect(screen.getByText('application/pdf')).toBeInTheDocument();
    expect(screen.getByText('2.0 KB')).toBeInTheDocument();
    expect(screen.getByText('delivered')).toBeInTheDocument();
  });

  test('falls back gracefully for a minimal artifact (id name, unknown type, no size)', async () => {
    vi.mocked(apiClient.artifacts.list).mockResolvedValue({ artifacts: [sampleArtifactMinimal] });
    await openArtifacts();
    expect(await screen.findByText('art-2')).toBeInTheDocument();
    expect(screen.getByText('unknown type')).toBeInTheDocument();
  });

  test('tolerates a response missing the artifacts field', async () => {
    vi.mocked(apiClient.artifacts.list).mockResolvedValue({} as { artifacts: never[] });
    await openArtifacts();
    expect(await screen.findByText('No artifacts yet.')).toBeInTheDocument();
  });

  test('shows the error state on rejection', async () => {
    vi.mocked(apiClient.artifacts.list).mockRejectedValueOnce(new Error('artifacts down'));
    await openArtifacts();
    expect(await screen.findByText('Failed to load')).toBeInTheDocument();
  });

  test('shows payment-required on a PaymentRequiredError', async () => {
    vi.mocked(apiClient.artifacts.list).mockRejectedValueOnce(new PaymentRequiredError(null));
    await openArtifacts();
    expect(await screen.findByText('Access requires payment')).toBeInTheDocument();
  });

  test('shows the loading spinner while artifacts are pending', async () => {
    vi.mocked(apiClient.artifacts.list).mockReturnValue(new Promise(() => {}));
    await openArtifacts();
    expect(screen.getByText('Loading artifacts…')).toBeInTheDocument();
  });
});

// ── Status badge styling ──────────────────────────────────────────────────────

describe('status badges', () => {
  test('renders the section description header', () => {
    render(<MarketplaceSection />);
    expect(screen.getByText('Browse products, jobs, escrows, and artifacts')).toBeInTheDocument();
  });

  test('a known status uses its colour mapping (accepted)', async () => {
    vi.mocked(apiClient.escrow.list).mockResolvedValue({
      escrows: [{ ...activeEscrow, status: 'accepted' }],
    });
    render(<MarketplaceSection />);
    await userEvent.click(screen.getByRole('tab', { name: 'Active' }));
    const badge = await screen.findByText('accepted');
    expect(badge.className).toContain('primary');
  });
});

// ── Search tab — x402 buy flow ─────────────────────────────────────────────────

describe('Search tab — buy product (x402)', () => {
  beforeEach(() => {
    vi.mocked(apiClient.graphql.products).mockResolvedValue({ products: [sampleProduct] });
  });

  test('Buy → confirm dialog shows the challenge amount + balance', async () => {
    vi.mocked(apiClient.marketplace.buyProduct).mockResolvedValueOnce({
      challenge: { amount: '10000000', asset: 'USDC', network: 'solana-devnet' },
      walletBalance: { raw: '50000000', formatted: '50', decimals: 6, assetSymbol: 'USDC' },
      walletAddress: 'WalletAbc123456789',
    });
    render(<MarketplaceSection />);
    await userEvent.click(await screen.findByRole('button', { name: 'Buy' }));

    expect(await screen.findByTestId('x402-amount')).toHaveTextContent('10 USDC');
    expect(screen.getByTestId('x402-balance')).toHaveTextContent('50 USDC');
    expect(apiClient.marketplace.buyProduct).toHaveBeenCalledWith('prod-1', { confirmed: false });
  });

  test('Confirm & Pay → success banner with the explorer link', async () => {
    vi.mocked(apiClient.marketplace.buyProduct)
      .mockResolvedValueOnce({
        challenge: { amount: '10000000', asset: 'USDC', network: 'solana-devnet' },
        walletBalance: { raw: '50000000', formatted: '50', decimals: 6, assetSymbol: 'USDC' },
        walletAddress: 'WalletAbc123456789',
      })
      .mockResolvedValueOnce({ result: { purchaseId: 'p1' }, payment: { onChainTx: 'TxBuy1' } });
    render(<MarketplaceSection />);
    await userEvent.click(await screen.findByRole('button', { name: 'Buy' }));
    await userEvent.click(await screen.findByTestId('x402-confirm'));

    const success = await screen.findByTestId('buy-success');
    expect(success).toHaveTextContent('Purchased Widget Builder');
    expect(success.querySelector('a')).toHaveAttribute(
      'href',
      'https://explorer.solana.com/tx/TxBuy1?cluster=devnet'
    );
    expect(apiClient.marketplace.buyProduct).toHaveBeenLastCalledWith('prod-1', {
      confirmed: true,
    });
  });

  test('post-payment failure shows the broadcast tx', async () => {
    vi.mocked(apiClient.marketplace.buyProduct)
      .mockResolvedValueOnce({
        challenge: { amount: '10000000', asset: 'USDC', network: 'solana-devnet' },
        walletBalance: { raw: '50000000', formatted: '50', decimals: 6, assetSymbol: 'USDC' },
        walletAddress: 'WalletAbc123456789',
      })
      .mockRejectedValueOnce(new Error('purchase paid but not confirmed (onChainTx=BrokeTx7)'));
    render(<MarketplaceSection />);
    await userEvent.click(await screen.findByRole('button', { name: 'Buy' }));
    await userEvent.click(await screen.findByTestId('x402-confirm'));

    const err = await screen.findByTestId('buy-error');
    expect(err).toHaveTextContent('Payment sent but purchase did not complete.');
    expect(err.querySelector('a')).toHaveAttribute(
      'href',
      'https://explorer.solana.com/tx/BrokeTx7'
    );
  });

  test('Cancel closes the dialog without a confirmed buy', async () => {
    vi.mocked(apiClient.marketplace.buyProduct).mockResolvedValueOnce({
      challenge: { amount: '10000000', asset: 'USDC', network: 'solana-devnet' },
      walletBalance: { raw: '50000000', formatted: '50', decimals: 6, assetSymbol: 'USDC' },
      walletAddress: 'WalletAbc123456789',
    });
    render(<MarketplaceSection />);
    await userEvent.click(await screen.findByRole('button', { name: 'Buy' }));
    await userEvent.click(await screen.findByRole('button', { name: 'Cancel' }));
    expect(screen.queryByTestId('x402-confirm')).not.toBeInTheDocument();
    expect(apiClient.marketplace.buyProduct).toHaveBeenCalledTimes(1);
  });
});
