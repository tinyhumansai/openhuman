/**
 * Tests for WalletAddressChip — the always-visible wallet address chip in the
 * Agent World sidebar header.
 *
 * All addresses are GENERIC placeholders (never real wallet addresses).
 */
import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, test, vi } from 'vitest';

// Pull the mock reference after vi.mock is hoisted.
import { fetchWalletStatus } from '../../services/walletApi';
import WalletAddressChip from './WalletAddressChip';

// ---------------------------------------------------------------------------
// Module mock — walletApi
// ---------------------------------------------------------------------------

vi.mock('../../services/walletApi', () => ({ fetchWalletStatus: vi.fn() }));

const mockFetchWalletStatus = vi.mocked(fetchWalletStatus);

// Generic placeholder wallet status — 44-char base58 Solana address.
const SOLANA_ADDRESS = 'AAAAAA1111112222223333334444445555556789BB';

function makeStatus(address: string | null = SOLANA_ADDRESS) {
  return {
    configured: address !== null,
    onboardingCompleted: address !== null,
    consentGranted: true,
    secretStored: true,
    source: 'generated' as const,
    mnemonicWordCount: 12,
    accounts:
      address !== null
        ? [
            {
              chain: 'evm' as const,
              address: '0xGENERICevm0000',
              derivationPath: "m/44'/60'/0'/0/0",
            },
            { chain: 'solana' as const, address, derivationPath: "m/44'/501'/0'/0'" },
          ]
        : [],
    updatedAtMs: Date.now(),
  };
}

// ---------------------------------------------------------------------------
// Clipboard mock (jsdom doesn't implement navigator.clipboard)
// ---------------------------------------------------------------------------

const clipboardWriteText = vi.fn().mockResolvedValue(undefined);

beforeEach(() => {
  // Reset mock implementation between tests so a never-resolving promise from
  // one test doesn't bleed into the next (vitest config has mockReset: false).
  mockFetchWalletStatus.mockReset();
  // Clear clipboard call history so per-test assertions don't see stale calls.
  clipboardWriteText.mockClear();

  Object.defineProperty(navigator, 'clipboard', {
    value: { writeText: clipboardWriteText },
    writable: true,
    configurable: true,
  });
});

// ---------------------------------------------------------------------------
// Test suite
// ---------------------------------------------------------------------------

describe('WalletAddressChip', () => {
  test('renders a loading skeleton before wallet resolves', () => {
    // Never resolves during this test — promise stays pending.
    mockFetchWalletStatus.mockReturnValue(new Promise(() => {}));

    render(<WalletAddressChip />);

    const chip = screen.getByTestId('wallet-address-chip');
    expect(chip).toBeInTheDocument();
    // In loading state the chip is the pulse element itself (no address text).
    expect(chip).not.toHaveTextContent(SOLANA_ADDRESS);
    expect(chip.className).toContain('animate-pulse');
  });

  test('renders truncated address (6…4) in ready state', async () => {
    mockFetchWalletStatus.mockResolvedValue(makeStatus());

    render(<WalletAddressChip />);

    // Truncated: first 6 + last 4 of SOLANA_ADDRESS
    const expected = `${SOLANA_ADDRESS.slice(0, 6)}…${SOLANA_ADDRESS.slice(-4)}`;
    await screen.findByText(expected);

    const chip = screen.getByTestId('wallet-address-chip');
    expect(chip).toBeInTheDocument();
  });

  test('full address is in the title attribute of the truncated span', async () => {
    mockFetchWalletStatus.mockResolvedValue(makeStatus());

    render(<WalletAddressChip />);

    await waitFor(() => {
      const span = screen.getByTitle(SOLANA_ADDRESS);
      expect(span).toBeInTheDocument();
    });
  });

  test('copy button calls clipboard.writeText with the full address', async () => {
    mockFetchWalletStatus.mockResolvedValue(makeStatus());

    render(<WalletAddressChip />);

    // Wait for ready state.
    await screen.findByTitle(SOLANA_ADDRESS);

    const copyBtn = screen.getByRole('button', { name: /copy address/i });
    await userEvent.click(copyBtn);

    expect(clipboardWriteText).toHaveBeenCalledWith(SOLANA_ADDRESS);
  });

  test('copy button shows "Copied" feedback after click and reverts', async () => {
    // Fake timers so we can fast-forward the 2s reset; shouldAdvanceTime keeps
    // findBy/waitFor polling alive, and userEvent drives the same clock.
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
      mockFetchWalletStatus.mockResolvedValue(makeStatus());

      render(<WalletAddressChip />);
      await screen.findByTitle(SOLANA_ADDRESS);

      const copyBtn = screen.getByRole('button', { name: /copy address/i });
      await user.click(copyBtn);

      // After click the aria-label should flip to "Copied".
      await waitFor(() =>
        expect(screen.getByRole('button', { name: /copied/i })).toBeInTheDocument()
      );

      // After the 2s reset timer fires, it should revert to "Copy address".
      await act(async () => {
        vi.advanceTimersByTime(2000);
      });
      await waitFor(() =>
        expect(screen.getByRole('button', { name: /copy address/i })).toBeInTheDocument()
      );
    } finally {
      vi.useRealTimers();
    }
  });

  test('shows a retryable "Wallet unavailable" state when fetchWalletStatus rejects', async () => {
    // A transient RPC/transport failure must NOT be reported as "not set up" —
    // a configured wallet would otherwise be mislabelled until the route remounts.
    mockFetchWalletStatus.mockRejectedValue(new Error('core rpc unavailable'));

    render(<WalletAddressChip />);

    await screen.findByText(/wallet unavailable/i);

    const chip = screen.getByTestId('wallet-address-chip');
    expect(chip).not.toHaveTextContent(SOLANA_ADDRESS);
    // The error chip is itself a retry button — and it must NOT claim "not set up".
    expect(screen.queryByText(/wallet not set up/i)).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument();
  });

  test('error state recovers to ready after a successful retry', async () => {
    // First fetch fails (transient), the retry click succeeds.
    mockFetchWalletStatus
      .mockRejectedValueOnce(new Error('core rpc unavailable'))
      .mockResolvedValueOnce(makeStatus());

    render(<WalletAddressChip />);

    const retryBtn = await screen.findByRole('button', { name: /retry/i });
    await userEvent.click(retryBtn);

    // After the retry resolves, the truncated address is shown.
    await screen.findByTitle(SOLANA_ADDRESS);
    expect(screen.queryByText(/wallet unavailable/i)).not.toBeInTheDocument();
  });

  test('shows "Wallet not set up" when no solana account exists in accounts list', async () => {
    // Wallet configured but only EVM — no Solana entry.
    mockFetchWalletStatus.mockResolvedValue(makeStatus(null));

    render(<WalletAddressChip />);

    await screen.findByText(/wallet not set up/i);
    expect(screen.queryByRole('button')).not.toBeInTheDocument();
  });

  test('copy button has descriptive aria-label visible to assistive technology', async () => {
    mockFetchWalletStatus.mockResolvedValue(makeStatus());

    render(<WalletAddressChip />);
    await screen.findByTitle(SOLANA_ADDRESS);

    // The button must be discoverable by assistive technology.
    expect(screen.getByRole('button', { name: /copy address/i })).toBeInTheDocument();
  });
});
