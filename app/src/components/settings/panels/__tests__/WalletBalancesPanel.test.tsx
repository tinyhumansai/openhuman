import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { BalanceInfo, WalletStatus } from '../../../../services/walletApi';
import { renderWithProviders } from '../../../../test/test-utils';
import WalletBalancesPanel from '../WalletBalancesPanel';

// ---------------------------------------------------------------------------
// Module-level mock: replace the wallet API before the panel loads. The panel
// checks `fetchWalletStatus()` first (to detect the not-configured state) and
// only calls `fetchWalletBalances()` when the wallet is configured.
// ---------------------------------------------------------------------------

const mockFetchWalletBalances = vi.fn<() => Promise<BalanceInfo[]>>();
const mockFetchWalletStatus = vi.fn<() => Promise<WalletStatus>>();

vi.mock('../../../../services/walletApi', () => ({
  fetchWalletBalances: (...args: unknown[]) => mockFetchWalletBalances(...(args as [])),
  fetchWalletStatus: (...args: unknown[]) => mockFetchWalletStatus(...(args as [])),
  // Send modal imports these; not exercised by the open-modal tests below.
  prepareTransfer: vi.fn(),
  executePrepared: vi.fn(),
}));

// The Receive modal renders a QR code; stub it to a lightweight element.
vi.mock('qrcode.react', () => ({
  QRCodeSVG: ({ value }: { value: string }) => <div data-testid="qr-code" data-value={value} />,
}));

const mockNavigateToSettings = vi.fn();
vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: vi.fn(),
    navigateToSettings: mockNavigateToSettings,
    breadcrumbs: [],
  }),
}));

const CONFIGURED_STATUS: WalletStatus = {
  configured: true,
  onboardingCompleted: true,
  consentGranted: true,
  secretStored: true,
  source: 'generated',
  mnemonicWordCount: 12,
  accounts: [],
  updatedAtMs: 1,
};

const UNCONFIGURED_STATUS: WalletStatus = {
  configured: false,
  onboardingCompleted: false,
  consentGranted: false,
  secretStored: false,
  source: null,
  mnemonicWordCount: null,
  accounts: [],
  updatedAtMs: null,
};

// Default every test to a configured wallet; not-configured tests opt in via
// `mockFetchWalletStatus.mockResolvedValueOnce(UNCONFIGURED_STATUS)`.
beforeEach(() => {
  mockFetchWalletStatus.mockReset();
  mockFetchWalletStatus.mockResolvedValue(CONFIGURED_STATUS);
  mockFetchWalletBalances.mockReset();
  mockNavigateToSettings.mockReset();
});

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

const EVM_BALANCE: BalanceInfo = {
  chain: 'evm',
  evmNetwork: 'ethereum_mainnet',
  address: '0x9858EfFD232B4033E47d90003D41EC34EcaEda94',
  assetSymbol: 'ETH',
  decimals: 18,
  raw: '1000000000000000000',
  formatted: '1.000000000000000000',
  providerStatus: 'ready',
};

const BTC_BALANCE: BalanceInfo = {
  chain: 'btc',
  address: 'bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu',
  assetSymbol: 'BTC',
  decimals: 8,
  raw: '100000000',
  formatted: '1.00000000',
  providerStatus: 'ready',
};

const MISSING_PROVIDER_BALANCE: BalanceInfo = {
  chain: 'solana',
  address: 'HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk',
  assetSymbol: 'SOL',
  decimals: 9,
  raw: '0',
  formatted: '0.000000000',
  providerStatus: 'missing',
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function renderPanel() {
  const { container } = renderWithProviders(<WalletBalancesPanel />);
  return container;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('WalletBalancesPanel — loading state', () => {
  it('shows a loading spinner while the fetch is in progress', async () => {
    let resolve!: (value: BalanceInfo[]) => void;
    mockFetchWalletBalances.mockReturnValueOnce(
      new Promise<BalanceInfo[]>(res => {
        resolve = res;
      })
    );

    renderPanel();

    expect(screen.getByText(/loading balances/i)).toBeInTheDocument();

    // Resolve so React can clean up.
    resolve([]);
    await waitFor(() => expect(screen.queryByText(/loading balances/i)).not.toBeInTheDocument());
  });
});

describe('WalletBalancesPanel — error state', () => {
  beforeEach(() => {
    mockFetchWalletBalances.mockReset();
  });

  it('renders a translated, user-facing error message when the fetch rejects', async () => {
    mockFetchWalletBalances.mockRejectedValueOnce(
      new Error('wallet is not configured; run wallet setup first')
    );

    renderPanel();

    // UI must not leak raw backend phrasing — it should render the
    // translated `walletBalances.errorGeneric` copy instead.
    await waitFor(() => {
      expect(screen.getByText(/Unable to load wallet balances/i)).toBeInTheDocument();
      expect(
        screen.queryByText(/wallet is not configured; run wallet setup first/i)
      ).not.toBeInTheDocument();
    });
  });

  it('re-invokes fetchWalletBalances when the Retry button is clicked', async () => {
    mockFetchWalletBalances
      .mockRejectedValueOnce(new Error('network error'))
      .mockResolvedValueOnce([]);

    renderPanel();

    await waitFor(() => expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument());

    fireEvent.click(screen.getByRole('button', { name: /retry/i }));

    await waitFor(() => expect(mockFetchWalletBalances).toHaveBeenCalledTimes(2));
    // After the second call (empty) the error clears and empty state appears.
    await waitFor(() =>
      expect(screen.queryByRole('button', { name: /retry/i })).not.toBeInTheDocument()
    );
  });
});

describe('WalletBalancesPanel — empty state', () => {
  beforeEach(() => {
    mockFetchWalletBalances.mockReset();
  });

  it('renders the Recovery Phrase hint when no balances are returned', async () => {
    mockFetchWalletBalances.mockResolvedValueOnce([]);

    renderPanel();

    await waitFor(() => {
      expect(screen.getByText(/No wallet accounts yet/i)).toBeInTheDocument();
      expect(screen.getByText(/Recovery Phrase/i)).toBeInTheDocument();
    });
  });
});

describe('WalletBalancesPanel — wallet not configured', () => {
  it('shows the setup hint + placeholder rows instead of a blocking error', async () => {
    mockFetchWalletStatus.mockReset();
    mockFetchWalletStatus.mockResolvedValueOnce(UNCONFIGURED_STATUS);

    renderPanel();

    await waitFor(() => {
      expect(screen.getByText(/Set it up to enable your wallet/i)).toBeInTheDocument();
    });
    // Placeholder rows render per displayed network (Ethereum/Base/BNB Chain)
    // plus Bitcoin/Solana/Tron — one "Not set up" each.
    expect(screen.getByText('Ethereum')).toBeInTheDocument();
    expect(screen.getByText('Base')).toBeInTheDocument();
    expect(screen.getByText('BNB Chain')).toBeInTheDocument();
    expect(screen.getAllByText('Not set up')).toHaveLength(6);
    // No balances fetch, no red error / retry button.
    expect(mockFetchWalletBalances).not.toHaveBeenCalled();
    expect(screen.queryByRole('button', { name: /retry/i })).not.toBeInTheDocument();
  });

  it('routes to the recovery phrase panel from the setup CTA', async () => {
    mockFetchWalletStatus.mockReset();
    mockFetchWalletStatus.mockResolvedValueOnce(UNCONFIGURED_STATUS);

    renderPanel();

    const cta = await screen.findByRole('button', { name: /set up recovery phrase/i });
    fireEvent.click(cta);

    expect(mockNavigateToSettings).toHaveBeenCalledWith('recovery-phrase');
  });
});

describe('WalletBalancesPanel — loaded state', () => {
  beforeEach(() => {
    mockFetchWalletBalances.mockReset();
  });

  it('renders chain badge, formatted amount, and symbol for each row', async () => {
    mockFetchWalletBalances.mockResolvedValueOnce([EVM_BALANCE, BTC_BALANCE]);

    renderPanel();

    await waitFor(() => {
      // EVM rows now show the network label + a per-network badge.
      expect(screen.getByText('Ethereum')).toBeInTheDocument();
      expect(screen.getByText('Bitcoin')).toBeInTheDocument();
      // Formatted balances (unique per row)
      expect(screen.getByText('1.000000000000000000')).toBeInTheDocument();
      expect(screen.getByText('1.00000000')).toBeInTheDocument();
      // ETH appears as the EVM badge + asset symbol; BTC as the badge + symbol.
      expect(screen.getAllByText('ETH').length).toBeGreaterThanOrEqual(2);
      expect(screen.getAllByText('BTC').length).toBeGreaterThanOrEqual(2);
    });
  });

  it('truncates addresses to first 6 + last 4 chars', async () => {
    mockFetchWalletBalances.mockResolvedValueOnce([EVM_BALANCE]);

    renderPanel();

    // address: 0x9858EfFD232B4033E47d90003D41EC34EcaEda94
    // truncated: 0x9858…da94 (first 6 + last 4 chars, original case preserved)
    await waitFor(() => {
      expect(screen.getByText('0x9858…da94')).toBeInTheDocument();
    });
  });

  it('shows the "provider unavailable" chip for balances with missing provider status', async () => {
    mockFetchWalletBalances.mockResolvedValueOnce([MISSING_PROVIDER_BALANCE]);

    renderPanel();

    await waitFor(() => {
      expect(screen.getByText(/provider unavailable/i)).toBeInTheDocument();
    });
  });

  it('does NOT show the provider chip for balances with ready status', async () => {
    mockFetchWalletBalances.mockResolvedValueOnce([EVM_BALANCE]);

    renderPanel();

    await waitFor(() => {
      expect(screen.queryByText(/provider unavailable/i)).not.toBeInTheDocument();
    });
  });
});

describe('WalletBalancesPanel — send / receive actions', () => {
  beforeEach(() => {
    mockFetchWalletBalances.mockReset();
  });

  it('opens the Receive modal with the row address + QR when Receive is clicked', async () => {
    mockFetchWalletBalances.mockResolvedValueOnce([EVM_BALANCE]);

    renderPanel();

    await waitFor(() => expect(screen.getByText('Receive')).toBeInTheDocument());
    fireEvent.click(screen.getByText('Receive'));

    expect(screen.getByTestId('receive-address')).toHaveTextContent(EVM_BALANCE.address);
    expect(screen.getByTestId('qr-code')).toHaveAttribute('data-value', EVM_BALANCE.address);
  });

  it('opens the Send modal with the recipient + amount fields when Send is clicked', async () => {
    mockFetchWalletBalances.mockResolvedValueOnce([EVM_BALANCE]);

    renderPanel();

    await waitFor(() => expect(screen.getByText('Send')).toBeInTheDocument());
    fireEvent.click(screen.getByText('Send'));

    expect(screen.getByTestId('send-recipient')).toBeInTheDocument();
    expect(screen.getByTestId('send-amount')).toBeInTheDocument();
    expect(screen.getByTestId('send-review')).toBeInTheDocument();
  });
});

describe('WalletBalancesPanel — refresh', () => {
  beforeEach(() => {
    mockFetchWalletBalances.mockReset();
  });

  it('re-invokes fetchWalletBalances when Refresh is clicked', async () => {
    mockFetchWalletBalances
      .mockResolvedValueOnce([EVM_BALANCE])
      .mockResolvedValueOnce([EVM_BALANCE, BTC_BALANCE]);

    renderPanel();

    await waitFor(() => expect(screen.getByText('Ethereum')).toBeInTheDocument());

    const refreshButton = screen.getByRole('button', { name: /refresh/i });
    fireEvent.click(refreshButton);

    await waitFor(() => expect(mockFetchWalletBalances).toHaveBeenCalledTimes(2));
    // After refresh, the BTC row is added — BTC appears twice (chain badge + symbol).
    await waitFor(() => expect(screen.getAllByText('BTC').length).toBeGreaterThanOrEqual(2));
  });
});
