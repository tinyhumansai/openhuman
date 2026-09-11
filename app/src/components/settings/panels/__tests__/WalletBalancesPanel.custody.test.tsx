import { fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { BalanceInfo, WalletStatus } from '../../../../services/walletApi';
import { renderWithProviders } from '../../../../test/test-utils';
import WalletBalancesPanel from '../WalletBalancesPanel';

/**
 * Paths in `WalletBalancesPanel` the existing suite does not reach. That suite
 * covers loading/error/retry, the not-configured placeholder state, row
 * rendering, address truncation and opening the Send/Receive modals; measured,
 * it still leaves the panel at 83.1% lines / 72.9% branches / 70.8% functions.
 *
 * What is left is the part that decides whether the user is looking at the
 * right address and the right numbers:
 *
 *   - `handleCopyAddress` (panel :104-118) — the whole function, plus its 2s
 *     reset timer and the re-copy path that clears a pending timer.
 *   - the unmount cleanup that clears that timer (:94-101).
 *   - `truncateAddress`'s short-address guard (:55).
 *
 * Deliberately NOT covered here:
 *   - the `requestId !== latestRequestIdRef.current` staleness guards
 *     (:292, :300, :303). Two loads cannot overlap from the UI: the Refresh
 *     button is `disabled={loading}` (:482) and `loadBalances` is the only
 *     caller, so the guard is defensive and unreachable. Recorded in the
 *     findings file rather than forced with a non-UI harness.
 *   - the Send/Receive modal `onClose` / `onSuccess` props (:506, :510). The
 *     existing suite already opens both modals; the remainder is one-line prop
 *     plumbing into a shared Modal.
 */

const mockFetchWalletBalances = vi.fn<() => Promise<BalanceInfo[]>>();
const mockFetchWalletStatus = vi.fn<() => Promise<WalletStatus>>();

vi.mock('../../../../services/walletApi', () => ({
  fetchWalletBalances: (...args: unknown[]) => mockFetchWalletBalances(...(args as [])),
  fetchWalletStatus: (...args: unknown[]) => mockFetchWalletStatus(...(args as [])),
  prepareTransfer: vi.fn(),
  executePrepared: vi.fn(),
}));

vi.mock('qrcode.react', () => ({
  QRCodeSVG: ({ value }: { value: string }) => <div data-testid="qr-code" data-value={value} />,
}));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: vi.fn(),
    navigateToSettings: vi.fn(),
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

const LONG_ADDRESS = '0x9858EfFD232B4033E47d90003D41EC34EcaEda94';
/** 12 characters exactly — the boundary `truncateAddress` must not shorten. */
const SHORT_ADDRESS = 'TRXshort1234';

const evmBalance = (over: Partial<BalanceInfo> = {}): BalanceInfo => ({
  chain: 'evm',
  evmNetwork: 'ethereum_mainnet',
  address: LONG_ADDRESS,
  assetSymbol: 'ETH',
  decimals: 18,
  raw: '1000000000000000000',
  formatted: '1.000000000000000000',
  providerStatus: 'ready',
  ...over,
});

function installClipboard(fail = false) {
  const writeText = vi.fn(async () => {
    if (fail) throw new Error('clipboard blocked');
    return undefined;
  });
  Object.assign(navigator, { clipboard: { writeText } });
  return writeText;
}

const copyButtons = () => screen.getAllByRole('button', { name: /copy address/i });

beforeEach(() => {
  vi.clearAllMocks();
  mockFetchWalletStatus.mockResolvedValue(CONFIGURED_STATUS);
  mockFetchWalletBalances.mockResolvedValue([evmBalance()]);
  installClipboard();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('WalletBalancesPanel — copying a receive address', () => {
  it('writes the full, untruncated address to the clipboard', async () => {
    // The row shows `0x9858…da94`; copying that would send funds nowhere.
    const writeText = installClipboard();
    renderWithProviders(<WalletBalancesPanel />);
    await waitFor(() => expect(copyButtons().length).toBeGreaterThan(0));

    fireEvent.click(copyButtons()[0]);
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(LONG_ADDRESS));
  });

  // Regression guard rather than a mutation-proven test: each row is its own
  // component instance holding its own `balance` prop, so this cannot be broken
  // by a realistic single-line change to the panel. Kept because it pins the
  // per-row wiring if the rows are ever refactored onto a shared handler.
  it('copies each row its own address, not the first row’s', async () => {
    const second = evmBalance({
      chain: 'btc',
      evmNetwork: undefined,
      address: 'bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu',
      assetSymbol: 'BTC',
    });
    mockFetchWalletBalances.mockResolvedValue([evmBalance(), second]);
    const writeText = installClipboard();

    renderWithProviders(<WalletBalancesPanel />);
    await waitFor(() => expect(copyButtons()).toHaveLength(2));

    fireEvent.click(copyButtons()[1]);
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(second.address));
  });

  it('does not claim success when the clipboard write is rejected', async () => {
    // `setCopied(true)` sits after the await, so a rejection must skip it.
    // Telling the user an address was copied when it was not is how funds go
    // to a stale address pasted from somewhere else.
    const writeText = installClipboard(true);
    renderWithProviders(<WalletBalancesPanel />);
    await waitFor(() => expect(copyButtons().length).toBeGreaterThan(0));

    const before = copyButtons()[0].innerHTML;
    fireEvent.click(copyButtons()[0]);
    await waitFor(() => expect(writeText).toHaveBeenCalled());
    expect(copyButtons()[0].innerHTML).toBe(before);
  });

  it('clears the copied indicator after two seconds', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    renderWithProviders(<WalletBalancesPanel />);
    await waitFor(() => expect(copyButtons().length).toBeGreaterThan(0));

    const before = copyButtons()[0].innerHTML;
    fireEvent.click(copyButtons()[0]);
    await waitFor(() => expect(copyButtons()[0].innerHTML).not.toBe(before));

    await vi.advanceTimersByTimeAsync(2100);
    await waitFor(() => expect(copyButtons()[0].innerHTML).toBe(before));
  });

  it('restarts the reset window when the address is copied twice', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    renderWithProviders(<WalletBalancesPanel />);
    await waitFor(() => expect(copyButtons().length).toBeGreaterThan(0));

    const before = copyButtons()[0].innerHTML;
    fireEvent.click(copyButtons()[0]);
    await waitFor(() => expect(copyButtons()[0].innerHTML).not.toBe(before));

    // 1.5s in, copy again: the first timer must be cleared, not left to fire.
    await vi.advanceTimersByTimeAsync(1500);
    fireEvent.click(copyButtons()[0]);

    // At 1.0s after the second copy the original 2s deadline has passed; if the
    // first timer had survived, the indicator would already be gone.
    await vi.advanceTimersByTimeAsync(1000);
    expect(copyButtons()[0].innerHTML).not.toBe(before);
  });

  it('does not fire the reset timer after the panel unmounts', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { unmount } = renderWithProviders(<WalletBalancesPanel />);
    await waitFor(() => expect(copyButtons().length).toBeGreaterThan(0));

    const before = copyButtons()[0].innerHTML;
    fireEvent.click(copyButtons()[0]);
    // `handleCopyAddress` is async, so the timer is scheduled a microtask later.
    await waitFor(() => expect(copyButtons()[0].innerHTML).not.toBe(before));
    expect(vi.getTimerCount()).toBeGreaterThan(0);
    unmount();

    // The cleanup effect (panel :94-101) must clear the pending reset timer;
    // left alone it would setState on an unmounted tree.
    expect(vi.getTimerCount()).toBe(0);
  });
});

describe('WalletBalancesPanel — address truncation boundary', () => {
  it('leaves a 12-character address intact rather than mangling it', async () => {
    mockFetchWalletBalances.mockResolvedValue([
      evmBalance({ chain: 'tron', evmNetwork: undefined, address: SHORT_ADDRESS }),
    ]);
    renderWithProviders(<WalletBalancesPanel />);

    expect(await screen.findByText(SHORT_ADDRESS)).toBeInTheDocument();
    // No ellipsis form of the same address.
    expect(screen.queryByText(/TRXsho…1234/)).not.toBeInTheDocument();
  });

  it('still truncates an address longer than the boundary', async () => {
    renderWithProviders(<WalletBalancesPanel />);
    expect(await screen.findByText(/^0x9858…da94$/)).toBeInTheDocument();
    expect(screen.queryByText(LONG_ADDRESS)).not.toBeInTheDocument();
  });
});
