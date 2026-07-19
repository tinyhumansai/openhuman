/**
 * Tests for LedgerSection — the Agent World Ledger section.
 *
 * Covers loading / error / empty / populated states, StatusBadge colors,
 * explorer links, address abbreviation, and inline expand/collapse.
 *
 * apiClient is mocked at module level; no real RPC calls are made.
 * All sample data uses generic placeholder names/IDs per project rules.
 */
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { type GqlLedgerTransaction } from '../../lib/agentworld/invokeApiClient';
import { apiClient } from '../AgentWorldShell';
import LedgerSection, {
  abbreviateAddress,
  formatAmount,
  formatLedgerAmount,
  LEDGER_PAGE_SIZE,
  StatusBadge,
} from './LedgerSection';

vi.mock('../AgentWorldShell', () => ({
  apiClient: { graphql: { ledgerTransactions: vi.fn(), ledgerTransaction: vi.fn() } },
}));

// ── Sample data (generic placeholders) ───────────────────────────────────────

const sampleTransaction: GqlLedgerTransaction = {
  txId: 'tx-001',
  visibility: 'unshielded',
  type: 'REGISTRATION',
  from: 'AAAA1111bbbb2222cccc3333dddd4444eeee5555',
  to: 'FFFF6666gggg7777hhhh8888iiii9999jjjj0000',
  // Smallest base unit (USDC has 6 decimals): 500000 micro-USDC = 0.50 USDC.
  amount: '500000',
  asset: 'USDC',
  network: 'solana-devnet',
  timestamp: '2026-06-01T12:00:00Z',
  onChainTx: '5wHu1qwD7q4H1x9b4g5v3z8k2m1n6p0r',
  status: 'SETTLED',
  reference: { kind: 'identity.register', id: 'ref-1' },
  metadata: { identity: '@test-agent' },
};

/** Build `n` distinct SALE rows with sequential ids, starting at `start`. */
function buildPage(n: number, start = 0): Array<GqlLedgerTransaction> {
  return Array.from({ length: n }, (_, i) => ({
    ...sampleTransaction,
    txId: `tx-${String(start + i).padStart(4, '0')}`,
    type: 'SALE',
  }));
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(apiClient.graphql.ledgerTransactions).mockResolvedValue({ transactions: [], count: 0 });
});

// ── Ledger list ───────────────────────────────────────────────────────────────

describe('Ledger list', () => {
  test('shows loading state before fetch resolves', () => {
    vi.mocked(apiClient.graphql.ledgerTransactions).mockReturnValue(new Promise(() => {}));
    render(<LedgerSection />);
    expect(screen.getByText(/loading ledger/i)).toBeInTheDocument();
  });

  test('shows empty state when ledger has no transactions', async () => {
    vi.mocked(apiClient.graphql.ledgerTransactions).mockResolvedValue({
      transactions: [],
      count: 0,
    });
    render(<LedgerSection />);
    await waitFor(() => {
      expect(screen.getByText(/no transactions found/i)).toBeInTheDocument();
    });
  });

  test('renders transaction list with type, amount, status, explorer link', async () => {
    vi.mocked(apiClient.graphql.ledgerTransactions).mockResolvedValue({
      transactions: [sampleTransaction],
      count: 1,
    });
    render(<LedgerSection />);
    await waitFor(() => {
      expect(screen.getByText('REGISTRATION')).toBeInTheDocument();
    });
    // 500000 micro-USDC scaled to display units → 0.5 USDC (trailing zero trimmed).
    expect(screen.getByText('0.5 USDC')).toBeInTheDocument();
    expect(screen.getByText('SETTLED')).toBeInTheDocument();
    expect(screen.getByText('View on chain')).toBeInTheDocument();
    // Network shown as a friendly label, not the raw genesis hash.
    expect(screen.getByText('Solana (devnet)')).toBeInTheDocument();
  });

  test('shows generic error on rejection', async () => {
    vi.mocked(apiClient.graphql.ledgerTransactions).mockRejectedValue(new Error('network failure'));
    render(<LedgerSection />);
    await waitFor(() => {
      expect(screen.getByText(/failed to load ledger/i)).toBeInTheDocument();
      expect(screen.getByText(/network failure/i)).toBeInTheDocument();
    });
  });

  test('tolerates response missing transactions field and shows empty', async () => {
    vi.mocked(apiClient.graphql.ledgerTransactions).mockResolvedValue({} as any);
    render(<LedgerSection />);
    await waitFor(() => {
      expect(screen.getByText(/no transactions found/i)).toBeInTheDocument();
    });
  });
});

// ── Pagination ────────────────────────────────────────────────────────────────

describe('Ledger pagination', () => {
  test('requests the first page with limit + offset 0', async () => {
    vi.mocked(apiClient.graphql.ledgerTransactions).mockResolvedValue({
      transactions: [sampleTransaction],
      count: 1,
    });
    render(<LedgerSection />);
    await waitFor(() => {
      expect(screen.getByText('REGISTRATION')).toBeInTheDocument();
    });
    expect(apiClient.graphql.ledgerTransactions).toHaveBeenNthCalledWith(1, {
      limit: LEDGER_PAGE_SIZE,
      offset: 0,
    });
  });

  test('hides Load more when the first page is shorter than a full page', async () => {
    vi.mocked(apiClient.graphql.ledgerTransactions).mockResolvedValue({
      transactions: buildPage(3),
      count: 3,
    });
    render(<LedgerSection />);
    await waitFor(() => {
      expect(screen.getAllByText('SALE')).toHaveLength(3);
    });
    expect(screen.queryByRole('button', { name: /load more/i })).not.toBeInTheDocument();
  });

  test('shows Load more when the first page fills the page size', async () => {
    vi.mocked(apiClient.graphql.ledgerTransactions).mockResolvedValue({
      transactions: buildPage(LEDGER_PAGE_SIZE),
      count: LEDGER_PAGE_SIZE,
    });
    render(<LedgerSection />);
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /load more/i })).toBeInTheDocument();
    });
  });

  test('clicking Load more fetches the next offset, appends rows, then stops', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.graphql.ledgerTransactions)
      .mockResolvedValueOnce({ transactions: buildPage(LEDGER_PAGE_SIZE, 0), count: 53 })
      .mockResolvedValueOnce({ transactions: buildPage(3, LEDGER_PAGE_SIZE), count: 53 });

    render(<LedgerSection />);
    await waitFor(() => {
      expect(screen.getAllByText('SALE')).toHaveLength(LEDGER_PAGE_SIZE);
    });

    await user.click(screen.getByRole('button', { name: /load more/i }));

    // Second page appended (50 + 3 = 53 rows) and the control disappears because
    // the short page signals the ledger is exhausted.
    await waitFor(() => {
      expect(screen.getAllByText('SALE')).toHaveLength(LEDGER_PAGE_SIZE + 3);
    });
    expect(apiClient.graphql.ledgerTransactions).toHaveBeenNthCalledWith(2, {
      limit: LEDGER_PAGE_SIZE,
      offset: LEDGER_PAGE_SIZE,
    });
    expect(screen.queryByRole('button', { name: /load more/i })).not.toBeInTheDocument();
  });

  test('deduplicates overlapping rows across pages', async () => {
    const user = userEvent.setup();
    // Second page repeats the last id of the first page (tx-0049) plus one new row.
    vi.mocked(apiClient.graphql.ledgerTransactions)
      .mockResolvedValueOnce({ transactions: buildPage(LEDGER_PAGE_SIZE, 0), count: 60 })
      .mockResolvedValueOnce({
        transactions: buildPage(LEDGER_PAGE_SIZE, LEDGER_PAGE_SIZE - 1),
        count: 60,
      });

    render(<LedgerSection />);
    await waitFor(() => {
      expect(screen.getAllByText('SALE')).toHaveLength(LEDGER_PAGE_SIZE);
    });

    await user.click(screen.getByRole('button', { name: /load more/i }));

    // 50 initial + 50 returned − 1 overlapping (tx-0049) = 99 unique rows.
    await waitFor(() => {
      expect(screen.getAllByText('SALE')).toHaveLength(2 * LEDGER_PAGE_SIZE - 1);
    });
  });

  test('keeps rows and surfaces an error when Load more fails', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.graphql.ledgerTransactions)
      .mockResolvedValueOnce({ transactions: buildPage(LEDGER_PAGE_SIZE, 0), count: 99 })
      .mockRejectedValueOnce(new Error('network failure'));

    render(<LedgerSection />);
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /load more/i })).toBeInTheDocument();
    });

    await user.click(screen.getByRole('button', { name: /load more/i }));

    // Existing rows stay; an error message appears; the control remains for retry.
    await waitFor(() => {
      expect(screen.getByText(/could not load more transactions/i)).toBeInTheDocument();
    });
    expect(screen.getAllByText('SALE')).toHaveLength(LEDGER_PAGE_SIZE);
    expect(screen.getByRole('button', { name: /load more/i })).toBeInTheDocument();
  });
});

// ── StatusBadge ───────────────────────────────────────────────────────────────

describe('StatusBadge colors', () => {
  test('status badge renders correct color for SETTLED', () => {
    render(<StatusBadge status="SETTLED" />);
    const badge = screen.getByText('SETTLED');
    expect(badge.className).toContain('green');
  });

  test('status badge renders correct color for PENDING', () => {
    render(<StatusBadge status="PENDING" />);
    const badge = screen.getByText('PENDING');
    expect(badge.className).toContain('amber');
  });

  test('status badge renders correct color for FAILED', () => {
    render(<StatusBadge status="FAILED" />);
    const badge = screen.getByText('FAILED');
    expect(badge.className).toContain('red');
  });
});

// ── Explorer link ─────────────────────────────────────────────────────────────

describe('Explorer link', () => {
  test('explorer link points to devnet for devnet network', async () => {
    vi.mocked(apiClient.graphql.ledgerTransactions).mockResolvedValue({
      transactions: [sampleTransaction],
      count: 1,
    });
    render(<LedgerSection />);
    await waitFor(() => {
      expect(screen.getByText('View on chain')).toBeInTheDocument();
    });
    const link = screen.getByText('View on chain').closest('a');
    expect(link?.href).toContain('?cluster=devnet');
  });

  test('explorer link points to mainnet for non-devnet network', async () => {
    const mainnetTx: GqlLedgerTransaction = {
      ...sampleTransaction,
      txId: 'tx-mainnet',
      network: 'solana-mainnet',
    };
    vi.mocked(apiClient.graphql.ledgerTransactions).mockResolvedValue({
      transactions: [mainnetTx],
      count: 1,
    });
    render(<LedgerSection />);
    await waitFor(() => {
      expect(screen.getByText('View on chain')).toBeInTheDocument();
    });
    const link = screen.getByText('View on chain').closest('a');
    expect(link?.href).not.toContain('cluster');
  });
});

// ── Inline expand ─────────────────────────────────────────────────────────────

describe('Inline expand', () => {
  test('click expands transaction to show full details', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.graphql.ledgerTransactions).mockResolvedValue({
      transactions: [sampleTransaction],
      count: 1,
    });
    render(<LedgerSection />);

    await waitFor(() => {
      expect(screen.getByText('REGISTRATION')).toBeInTheDocument();
    });

    // Before expansion: full from/to addresses not visible in detail pane
    expect(screen.queryByText('tx-001')).not.toBeInTheDocument();

    // Click row to expand
    await user.click(screen.getByText('REGISTRATION'));

    // Expanded: full txId and from address visible
    await waitFor(() => {
      expect(screen.getByText('tx-001')).toBeInTheDocument();
    });
    expect(screen.getByText('identity.register')).toBeInTheDocument();
    expect(screen.getByText('@test-agent')).toBeInTheDocument();
  });
});

// ── Address abbreviation ──────────────────────────────────────────────────────

describe('abbreviateAddress', () => {
  test('abbreviated addresses display correctly', () => {
    const addr = 'AAAA1111bbbb2222cccc3333dddd4444eeee5555';
    expect(abbreviateAddress(addr)).toBe('AAAA…5555');
  });

  test('handles missing from/to addresses', () => {
    expect(abbreviateAddress(undefined)).toBe('—');
    expect(abbreviateAddress('')).toBe('—');
  });

  test('returns short addresses unchanged', () => {
    expect(abbreviateAddress('short')).toBe('short');
    expect(abbreviateAddress('exactly12ch')).toBe('exactly12ch');
  });
});

describe('formatAmount', () => {
  test('groups large integers with thousands separators', () => {
    expect(formatAmount('1000000')).toBe('1,000,000');
    expect(formatAmount('500')).toBe('500');
  });

  test('preserves original decimal places', () => {
    expect(formatAmount('0.50')).toBe('0.50');
    expect(formatAmount('1234.5')).toBe('1,234.5');
  });

  test('passes through non-numeric and empty', () => {
    expect(formatAmount(undefined)).toBe('—');
    expect(formatAmount('n/a')).toBe('n/a');
  });
});

describe('formatLedgerAmount', () => {
  test('scales USDC base units (6 decimals) to display units', () => {
    // Regression: amounts were shown raw, reading ~1,000,000× too large.
    expect(formatLedgerAmount('1000000', 'USDC')).toBe('1');
    expect(formatLedgerAmount('500000', 'USDC')).toBe('0.5');
    expect(formatLedgerAmount('123456789', 'USDC')).toBe('123.456789');
  });

  test('scales SOL base units (9 decimals) and groups the integer part', () => {
    expect(formatLedgerAmount('2500000000', 'SOL')).toBe('2.5');
    expect(formatLedgerAmount('1000000000000', 'SOL')).toBe('1,000');
  });

  test('preserves the sign for debits', () => {
    expect(formatLedgerAmount('-500000', 'USDC')).toBe('-0.5');
  });

  test('leaves unknown / zero-decimal assets unscaled', () => {
    expect(formatLedgerAmount('42', 'POINTS')).toBe('42');
    expect(formatLedgerAmount('1000000', undefined)).toBe('1,000,000');
  });

  test('passes through empty', () => {
    expect(formatLedgerAmount(undefined, 'USDC')).toBe('—');
  });
});
