import { fireEvent, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import ConnectionsPanel from '../ConnectionsPanel';

const navigateMock = vi.fn();

vi.mock('react-router-dom', async () => {
  const actual = await vi.importActual<typeof import('react-router-dom')>('react-router-dom');
  return { ...actual, useNavigate: () => navigateMock };
});

const fetchWalletStatusMock = vi.fn();

vi.mock('../../../../services/walletApi', () => ({
  fetchWalletStatus: () => fetchWalletStatusMock(),
}));

const sampleConfigured = {
  configured: true,
  onboardingCompleted: true,
  consentGranted: true,
  secretStored: true,
  source: 'generated' as const,
  mnemonicWordCount: 12,
  accounts: [
    { chain: 'evm', address: '0xabc', derivationPath: "m/44'/60'/0'/0/0" },
    { chain: 'btc', address: 'bc1q', derivationPath: "m/44'/0'/0'/0/0" },
    { chain: 'solana', address: 'So1', derivationPath: "m/44'/501'/0'/0'" },
    { chain: 'tron', address: 'TR0', derivationPath: "m/44'/195'/0'/0/0" },
  ],
  updatedAtMs: 1234567890,
};

const sampleUnconfigured = {
  configured: false,
  onboardingCompleted: false,
  consentGranted: false,
  secretStored: false,
  source: null,
  mnemonicWordCount: null,
  accounts: [],
  updatedAtMs: null,
};

describe('ConnectionsPanel — trust-surface polish', () => {
  it('shows "Coming soon" badge on the three not-yet-shipped options (Web3 Wallet is now wired)', async () => {
    fetchWalletStatusMock.mockResolvedValueOnce(sampleUnconfigured);
    renderWithProviders(<ConnectionsPanel />);
    await waitFor(() => expect(fetchWalletStatusMock).toHaveBeenCalled());
    expect(screen.getAllByText(/Coming soon/i)).toHaveLength(3);
  });
});

describe('ConnectionsPanel — wallet status branches', () => {
  it('renders a "Checking…" badge while wallet status is loading', () => {
    let resolve: ((value: typeof sampleUnconfigured) => void) | undefined;
    fetchWalletStatusMock.mockImplementationOnce(
      () =>
        new Promise(r => {
          resolve = r;
        })
    );
    renderWithProviders(<ConnectionsPanel />);
    expect(screen.getByText(/Checking…/i)).toBeTruthy();
    expect(screen.getByText(/Checking wallet status/i)).toBeTruthy();
    resolve?.(sampleUnconfigured);
  });

  it('renders the Configured badge and wallet identities when status reports configured', async () => {
    fetchWalletStatusMock.mockResolvedValueOnce(sampleConfigured);
    renderWithProviders(<ConnectionsPanel />);
    await waitFor(() => expect(screen.getByText('Configured')).toBeTruthy());
    expect(screen.getByText('Wallet identities')).toBeTruthy();
    expect(screen.getByText('evm')).toBeTruthy();
    expect(screen.getByText('btc')).toBeTruthy();
    expect(screen.getByText('solana')).toBeTruthy();
    expect(screen.getByText('tron')).toBeTruthy();
    expect(
      screen.getByText(/Local EVM, BTC, Solana, and Tron identities are configured/i)
    ).toBeTruthy();
  });

  it('renders the Set up CTA when status reports unconfigured', async () => {
    fetchWalletStatusMock.mockResolvedValueOnce(sampleUnconfigured);
    renderWithProviders(<ConnectionsPanel />);
    await waitFor(() => expect(screen.getByText('Set up')).toBeTruthy());
    expect(screen.getByText(/Set up local EVM, BTC, Solana, and Tron identities/i)).toBeTruthy();
  });

  it('renders the Unavailable badge when fetchWalletStatus rejects', async () => {
    fetchWalletStatusMock.mockRejectedValueOnce(new Error('network down'));
    renderWithProviders(<ConnectionsPanel />);
    await waitFor(() => expect(screen.getByText(/Unavailable/i)).toBeTruthy());
    expect(screen.getByText(/Could not check wallet status/i)).toBeTruthy();
  });

  it('navigates to the recovery-phrase panel when the wallet row is clicked', async () => {
    fetchWalletStatusMock.mockResolvedValueOnce(sampleUnconfigured);
    navigateMock.mockReset();
    renderWithProviders(<ConnectionsPanel />);
    await waitFor(() => expect(screen.getByText('Set up')).toBeTruthy());
    fireEvent.click(screen.getByRole('button', { name: /Web3 Wallet/i }));
    expect(navigateMock).toHaveBeenCalledWith('/settings/recovery-phrase');
  });
});
