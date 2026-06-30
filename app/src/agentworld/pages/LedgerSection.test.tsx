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
