/**
 * Tests for IdentitiesSection — the Agent World Identities screen.
 *
 * Three tabs (Register / Registry / Trading). All data flows through the
 * mocked `apiClient` so no real RPC/network calls are made. We exercise every
 * tab, every AsyncState branch (loading / error / payment_required / empty /
 * populated), the handle-availability input + Check flow, and the static
 * register placeholder.
 *
 * All handles / ids / prices below are GENERIC placeholders — no real names.
 */
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';

import {
  type DirectoryIdentityListingsResponse,
  PaymentRequiredError,
} from '../../lib/agentworld/invokeApiClient';
import { apiClient } from '../AgentWorldShell';
import IdentitiesSection from './IdentitiesSection';

// ── Mock apiClient ────────────────────────────────────────────────────────────
// Replace every namespace/method IdentitiesSection calls through.

vi.mock('../AgentWorldShell', () => ({
  apiClient: {
    graphql: { identityListings: vi.fn() },
    registry: { get: vi.fn(), register: vi.fn() },
    directoryIdentities: { list: vi.fn() },
    marketplace: {
      listIdentities: vi.fn(),
      identityFloor: vi.fn(),
      recent: vi.fn(),
      buyIdentity: vi.fn(),
      bid: vi.fn(),
      offer: vi.fn(),
    },
  },
}));

// Default happy-path resolutions so async hooks settle without unhandled
// rejections. Individual tests override per-case.
beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(apiClient.registry.get).mockResolvedValue({ available: true, name: '@placeholder' });
  vi.mocked(apiClient.directoryIdentities.list).mockResolvedValue({ identities: [] });
  vi.mocked(apiClient.marketplace.listIdentities).mockResolvedValue({ identities: [] });
  vi.mocked(apiClient.marketplace.identityFloor).mockResolvedValue({ price: undefined });
  vi.mocked(apiClient.marketplace.recent).mockResolvedValue({ sales: [] });
  vi.mocked(apiClient.registry.register).mockResolvedValue({
    identity: { username: '@placeholder' },
  });
  vi.mocked(apiClient.marketplace.buyIdentity).mockResolvedValue({ result: { saleId: 's1' } });
  vi.mocked(apiClient.marketplace.bid).mockResolvedValue({ result: {}, committed: true });
  vi.mocked(apiClient.marketplace.offer).mockResolvedValue({ result: {}, committed: true });
  vi.mocked(apiClient.graphql.identityListings).mockImplementation(params => {
    if (typeof params?.length === 'number') {
      return vi
        .mocked(apiClient.marketplace.identityFloor)(params.length)
        .then(floor => ({
          identities: floor.price
            ? [
                {
                  listingId: `floor-${params.length}`,
                  name: `@floor-${params.length}`,
                  price: floor.price,
                  updatedAt: '',
                },
              ]
            : [],
        }));
    }
    if (params?.limit === 20) {
      return vi.mocked(apiClient.directoryIdentities.list)(params);
    }
    return vi.mocked(apiClient.marketplace.listIdentities)({
      limit: params?.limit,
      status: 'active',
    });
  });
});

afterEach(() => {
  vi.restoreAllMocks();
});

// ── Helpers ─────────────────────────────────────────────────────────────────

function gotoTab(name: 'Register' | 'Registry' | 'Trading') {
  return userEvent.click(screen.getByRole('button', { name }));
}

// ── Tab navigation ────────────────────────────────────────────────────────────

describe('tab navigation', () => {
  test('defaults to Register tab', () => {
    render(<IdentitiesSection />);
    expect(screen.getByRole('button', { name: 'Register' })).toHaveAttribute('data-active', 'true');
    expect(screen.getByText('Check handle availability')).toBeInTheDocument();
  });

  test('renders the section description', () => {
    render(<IdentitiesSection />);
    expect(
      screen.getByText(/Claim handles, manage your registry, and trade identities/i)
    ).toBeInTheDocument();
  });

  test('can switch to Registry tab', async () => {
    render(<IdentitiesSection />);
    await gotoTab('Registry');
    expect(screen.getByRole('button', { name: 'Registry' })).toHaveAttribute('data-active', 'true');
    expect(await screen.findByText('Directory identities')).toBeInTheDocument();
  });

  test('can switch to Trading tab', async () => {
    render(<IdentitiesSection />);
    await gotoTab('Trading');
    expect(screen.getByRole('button', { name: 'Trading' })).toHaveAttribute('data-active', 'true');
    expect(await screen.findByText('Floor Prices')).toBeInTheDocument();
  });

  test('clicking the active tab again is a no-op (reducer short-circuit)', async () => {
    render(<IdentitiesSection />);
    await gotoTab('Register');
    expect(screen.getByRole('button', { name: 'Register' })).toHaveAttribute('data-active', 'true');
  });

  test('switching tabs remounts the body (key change clears local state)', async () => {
    render(<IdentitiesSection />);
    await gotoTab('Registry');
    await gotoTab('Register');
    // Back on Register, input should be fresh/empty.
    const input = screen.getByPlaceholderText('Search for a name...') as HTMLInputElement;
    expect(input.value).toBe('');
  });
});

// ── Register tab — handle availability ─────────────────────────────────────────

describe('Register tab — handle availability', () => {
  test('Check button is disabled when the input is empty', () => {
    render(<IdentitiesSection />);
    expect(screen.getByRole('button', { name: 'Check' })).toBeDisabled();
  });

  test('sanitizes input to lowercase a-z0-9_ only', async () => {
    render(<IdentitiesSection />);
    const input = screen.getByPlaceholderText('Search for a name...') as HTMLInputElement;
    await userEvent.type(input, 'Ab-C 1@!_d');
    expect(input.value).toBe('abc1_d');
  });

  test('shows "available" result when handle is free', async () => {
    vi.mocked(apiClient.registry.get).mockResolvedValue({ available: true, name: '@freehandle' });
    render(<IdentitiesSection />);
    await userEvent.type(screen.getByPlaceholderText('Search for a name...'), 'freehandle');
    await userEvent.click(screen.getByRole('button', { name: 'Check' }));

    expect(await screen.findByText('@freehandle is available')).toBeInTheDocument();
    expect(apiClient.registry.get).toHaveBeenCalledWith('@freehandle');
  });

  test('shows "taken" result with truncated cryptoId when handle is taken', async () => {
    vi.mocked(apiClient.registry.get).mockResolvedValue({
      available: false,
      name: '@takenhandle',
      identity: { cryptoId: 'abcdef0123456789deadbeef' },
    });
    render(<IdentitiesSection />);
    await userEvent.type(screen.getByPlaceholderText('Search for a name...'), 'takenhandle');
    await userEvent.click(screen.getByRole('button', { name: 'Check' }));

    expect(await screen.findByText('@takenhandle is taken')).toBeInTheDocument();
    // cryptoId truncated to first 12 chars + ellipsis
    expect(screen.getByText('abcdef012345...')).toBeInTheDocument();
  });

  test('shows "taken" result without cryptoId when identity is absent', async () => {
    vi.mocked(apiClient.registry.get).mockResolvedValue({ available: false, name: '@takenbare' });
    render(<IdentitiesSection />);
    await userEvent.type(screen.getByPlaceholderText('Search for a name...'), 'takenbare');
    await userEvent.click(screen.getByRole('button', { name: 'Check' }));

    expect(await screen.findByText('@takenbare is taken')).toBeInTheDocument();
  });

  test('shows error message when the availability check rejects with a plain Error', async () => {
    vi.mocked(apiClient.registry.get).mockRejectedValueOnce(new Error('boom-network'));
    render(<IdentitiesSection />);
    await userEvent.type(screen.getByPlaceholderText('Search for a name...'), 'errhandle');
    await userEvent.click(screen.getByRole('button', { name: 'Check' }));

    expect(await screen.findByText(/boom-network/)).toBeInTheDocument();
  });

  test('shows payment-required notice when the check rejects with PaymentRequiredError', async () => {
    vi.mocked(apiClient.registry.get).mockRejectedValueOnce(
      new PaymentRequiredError({ terms: 'x402' })
    );
    render(<IdentitiesSection />);
    await userEvent.type(screen.getByPlaceholderText('Search for a name...'), 'payhandle');
    await userEvent.click(screen.getByRole('button', { name: 'Check' }));

    expect(await screen.findByText('Payment required to check availability.')).toBeInTheDocument();
  });

  test('submitting via the form (Enter) also triggers the check', async () => {
    vi.mocked(apiClient.registry.get).mockResolvedValue({ available: true, name: '@viaenter' });
    render(<IdentitiesSection />);
    const input = screen.getByPlaceholderText('Search for a name...');
    await userEvent.type(input, 'viaenter{Enter}');
    expect(await screen.findByText('@viaenter is available')).toBeInTheDocument();
  });

  test('renders the pricing tiers', () => {
    render(<IdentitiesSection />);
    expect(screen.getByText('Pricing tiers')).toBeInTheDocument();
    expect(screen.getByText('$250/yr')).toBeInTheDocument();
    expect(screen.getByText('$50/yr')).toBeInTheDocument();
    expect(screen.getByText('$10/yr')).toBeInTheDocument();
  });

  test('shows a Register button only when the handle is available', async () => {
    vi.mocked(apiClient.registry.get).mockResolvedValue({ available: true, name: '@freehandle' });
    render(<IdentitiesSection />);
    await userEvent.type(screen.getByPlaceholderText('Search for a name...'), 'freehandle');
    await userEvent.click(screen.getByRole('button', { name: 'Check' }));
    expect(await screen.findByRole('button', { name: 'Register @freehandle' })).toBeInTheDocument();
  });
});

// ── Register tab — x402 registration flow ──────────────────────────────────────

const FREE_AMOUNT = '10000000'; // 10 USDC in base units (6 decimals)

async function checkAndRegister(handle: string) {
  await userEvent.type(screen.getByPlaceholderText('Search for a name...'), handle);
  await userEvent.click(screen.getByRole('button', { name: 'Check' }));
  await userEvent.click(await screen.findByRole('button', { name: `Register @${handle}` }));
}

describe('Register tab — x402 registration', () => {
  beforeEach(() => {
    vi.mocked(apiClient.registry.get).mockResolvedValue({ available: true, name: '@buyer' });
  });

  test('confirmed:false renders the confirm dialog with amount and balance', async () => {
    vi.mocked(apiClient.registry.register).mockResolvedValueOnce({
      challenge: { amount: FREE_AMOUNT, asset: 'USDC', network: 'solana-devnet' },
      walletBalance: { raw: '50000000', formatted: '50', decimals: 6, assetSymbol: 'USDC' },
      walletAddress: 'WaLLetdeadbeef0123456789',
    });
    render(<IdentitiesSection />);
    await checkAndRegister('buyer');

    expect(await screen.findByTestId('x402-amount')).toHaveTextContent('10 USDC');
    expect(screen.getByTestId('x402-balance')).toHaveTextContent('50 USDC');
    // confirmed:false probe was sent
    expect(apiClient.registry.register).toHaveBeenCalledWith({
      username: 'buyer',
      confirmed: false,
    });
    expect(screen.getByTestId('x402-confirm')).toBeEnabled();
  });

  test('insufficient balance swaps Pay for an Add-funds redirect', async () => {
    vi.mocked(apiClient.registry.register).mockResolvedValueOnce({
      challenge: { amount: FREE_AMOUNT, asset: 'USDC', network: 'solana-devnet' },
      walletBalance: { raw: '1000000', formatted: '1', decimals: 6, assetSymbol: 'USDC' },
      walletAddress: 'WaLLetdeadbeef0123456789',
    });
    render(<IdentitiesSection />);
    await checkAndRegister('buyer');

    expect(await screen.findByTestId('x402-insufficient')).toBeInTheDocument();
    // The disabled Pay button is replaced by an Add-funds action.
    expect(screen.queryByTestId('x402-confirm')).not.toBeInTheDocument();
    expect(screen.getByTestId('x402-add-funds')).toBeInTheDocument();
  });

  test('confirmed:true success renders the registered identity + explorer link', async () => {
    vi.mocked(apiClient.registry.register)
      .mockResolvedValueOnce({
        challenge: { amount: FREE_AMOUNT, asset: 'USDC', network: 'solana-devnet' },
        walletBalance: { raw: '50000000', formatted: '50', decimals: 6, assetSymbol: 'USDC' },
        walletAddress: 'WaLLetdeadbeef0123456789',
      })
      .mockResolvedValueOnce({
        identity: { username: '@buyer' },
        payment: { onChainTx: 'TxSig123456789' },
      });
    render(<IdentitiesSection />);
    await checkAndRegister('buyer');
    await userEvent.click(await screen.findByTestId('x402-confirm'));

    const success = await screen.findByTestId('register-success');
    expect(success).toHaveTextContent('Registered @buyer');
    const link = within(success).getByRole('link', { name: /Solana Explorer/ });
    expect(link).toHaveAttribute(
      'href',
      'https://explorer.solana.com/tx/TxSig123456789?cluster=devnet'
    );
    // The spend call carried confirmed:true.
    expect(apiClient.registry.register).toHaveBeenLastCalledWith({
      username: 'buyer',
      confirmed: true,
      actorType: 'human',
      primary: true,
    });
  });

  test('free-tier register (identity without challenge) short-circuits to success', async () => {
    vi.mocked(apiClient.registry.register).mockResolvedValueOnce({
      identity: { username: '@buyer' },
    });
    render(<IdentitiesSection />);
    await checkAndRegister('buyer');

    expect(await screen.findByTestId('register-success')).toHaveTextContent('Registered @buyer');
    // No confirm dialog appears for the free tier.
    expect(screen.queryByTestId('x402-confirm')).not.toBeInTheDocument();
  });

  test('post-payment failure surfaces the broadcast tx', async () => {
    vi.mocked(apiClient.registry.register)
      .mockResolvedValueOnce({
        challenge: { amount: FREE_AMOUNT, asset: 'USDC', network: 'solana-devnet' },
        walletBalance: { raw: '50000000', formatted: '50', decimals: 6, assetSymbol: 'USDC' },
        walletAddress: 'WaLLetdeadbeef0123456789',
      })
      .mockRejectedValueOnce(
        new Error('registration paid but not confirmed (onChainTx=BrokeTx99)')
      );
    render(<IdentitiesSection />);
    await checkAndRegister('buyer');
    await userEvent.click(await screen.findByTestId('x402-confirm'));

    const err = await screen.findByTestId('register-error');
    expect(err).toHaveTextContent('Payment sent but registration did not complete.');
    expect(within(err).getByRole('link', { name: /Solana Explorer/ })).toHaveAttribute(
      'href',
      'https://explorer.solana.com/tx/BrokeTx99'
    );
  });

  test('cancel closes the confirm dialog without spending', async () => {
    vi.mocked(apiClient.registry.register).mockResolvedValueOnce({
      challenge: { amount: FREE_AMOUNT, asset: 'USDC', network: 'solana-devnet' },
      walletBalance: { raw: '50000000', formatted: '50', decimals: 6, assetSymbol: 'USDC' },
      walletAddress: 'WaLLetdeadbeef0123456789',
    });
    render(<IdentitiesSection />);
    await checkAndRegister('buyer');

    await userEvent.click(await screen.findByRole('button', { name: 'Cancel' }));
    expect(screen.queryByTestId('x402-confirm')).not.toBeInTheDocument();
    // Only the confirmed:false probe ran — no spend.
    expect(apiClient.registry.register).toHaveBeenCalledTimes(1);
  });

  test('begin: an unexpected response (no identity, no challenge) errors out', async () => {
    vi.mocked(apiClient.registry.register).mockResolvedValueOnce({});
    render(<IdentitiesSection />);
    await checkAndRegister('buyer');
    expect(await screen.findByTestId('register-error')).toHaveTextContent('Unexpected response');
  });

  test('begin: a PaymentRequiredError surfaces a payment notice', async () => {
    vi.mocked(apiClient.registry.register).mockRejectedValueOnce(
      new PaymentRequiredError({ terms: 'x402' })
    );
    render(<IdentitiesSection />);
    await checkAndRegister('buyer');
    expect(await screen.findByTestId('register-error')).toHaveTextContent('Payment required.');
  });

  test('begin: a plain error surfaces its message', async () => {
    vi.mocked(apiClient.registry.register).mockRejectedValueOnce(new Error('probe-boom'));
    render(<IdentitiesSection />);
    await checkAndRegister('buyer');
    expect(await screen.findByTestId('register-error')).toHaveTextContent('probe-boom');
  });

  test('confirmPay: a response without an identity errors out', async () => {
    vi.mocked(apiClient.registry.register)
      .mockResolvedValueOnce({
        challenge: { amount: FREE_AMOUNT, asset: 'USDC', network: 'solana-devnet' },
        walletBalance: { raw: '50000000', formatted: '50', decimals: 6, assetSymbol: 'USDC' },
        walletAddress: 'WaLLetdeadbeef0123456789',
      })
      .mockResolvedValueOnce({});
    render(<IdentitiesSection />);
    await checkAndRegister('buyer');
    await userEvent.click(await screen.findByTestId('x402-confirm'));
    expect(await screen.findByTestId('register-error')).toHaveTextContent(
      'Registration did not complete.'
    );
  });
});

// ── Register tab — post-registration directory refresh ─────────────────────────
//
// After a successful registration (free-tier or paid), the parent increments
// `registryKey` via `bumpRegistryKey` → `RegistryTab` remounts → the directory
// fetch re-runs and surfaces the newly published agent card.

describe('Register tab — post-registration directory refresh', () => {
  beforeEach(() => {
    vi.mocked(apiClient.registry.get).mockResolvedValue({ available: true, name: '@buyer' });
  });

  test('free-tier success: Registry tab re-fetches and shows new card', async () => {
    // RegistryTab calls list() once per mount. After registration bumps the
    // registry key, it remounts and calls list() again with the updated response.
    vi.mocked(apiClient.directoryIdentities.list)
      .mockResolvedValueOnce({ identities: [] })
      .mockResolvedValueOnce({
        identities: [
          {
            listingId: 'new-id-1',
            name: '@buyer',
            price: { amount: '10', asset: 'USDC' },
            updatedAt: '2026-06-01T00:00:00Z',
            status: 'active',
          },
        ],
      });
    vi.mocked(apiClient.registry.register).mockResolvedValueOnce({
      identity: { username: '@buyer' },
    });

    render(<IdentitiesSection />);
    // Navigate to Registry first so it fetches the initial (empty) list.
    await gotoTab('Registry');
    expect(
      await screen.findByText(/No directory identities are currently listed/)
    ).toBeInTheDocument();

    // Go back to Register tab and trigger free-tier registration.
    await gotoTab('Register');
    await checkAndRegister('buyer');
    expect(await screen.findByTestId('register-success')).toBeInTheDocument();

    // Navigate to Registry tab again — RegistryTab remounts with new key,
    // re-fetches, and the new card appears.
    await gotoTab('Registry');
    expect(await screen.findByText('@buyer')).toBeInTheDocument();
    // list() was called twice: once on initial Registry mount, once on remount.
    expect(apiClient.directoryIdentities.list).toHaveBeenCalledTimes(2);
  });

  test('paid registration success: Registry tab re-fetches and shows new card', async () => {
    vi.mocked(apiClient.directoryIdentities.list)
      .mockResolvedValueOnce({ identities: [] })
      .mockResolvedValueOnce({
        identities: [
          {
            listingId: 'new-id-2',
            name: '@buyer',
            price: { amount: '10', asset: 'USDC' },
            updatedAt: '2026-06-01T00:00:00Z',
            status: 'active',
          },
        ],
      });
    vi.mocked(apiClient.registry.register)
      .mockResolvedValueOnce({
        challenge: { amount: FREE_AMOUNT, asset: 'USDC', network: 'solana-devnet' },
        walletBalance: { raw: '50000000', formatted: '50', decimals: 6, assetSymbol: 'USDC' },
        walletAddress: 'WaLLetdeadbeef0123456789',
      })
      .mockResolvedValueOnce({
        identity: { username: '@buyer' },
        payment: { onChainTx: 'TxSig999' },
      });

    render(<IdentitiesSection />);
    // Navigate to Registry first to consume the initial (empty) list response.
    await gotoTab('Registry');
    expect(
      await screen.findByText(/No directory identities are currently listed/)
    ).toBeInTheDocument();

    // Go back to Register tab and complete paid registration.
    await gotoTab('Register');
    await checkAndRegister('buyer');
    await userEvent.click(await screen.findByTestId('x402-confirm'));
    expect(await screen.findByTestId('register-success')).toBeInTheDocument();

    // Navigate to Registry tab — RegistryTab remounts and shows the new card.
    await gotoTab('Registry');
    expect(await screen.findByText('@buyer')).toBeInTheDocument();
    expect(apiClient.directoryIdentities.list).toHaveBeenCalledTimes(2);
  });

  test('failed registration does not bump registry key (no extra remount)', async () => {
    vi.mocked(apiClient.directoryIdentities.list).mockResolvedValue({ identities: [] });
    vi.mocked(apiClient.registry.register).mockRejectedValueOnce(new Error('probe-failed'));

    render(<IdentitiesSection />);
    // Trigger a failed registration.
    await checkAndRegister('buyer');
    expect(await screen.findByTestId('register-error')).toBeInTheDocument();

    // Navigate to Registry tab — only one list() call (normal mount, no extra bump).
    await gotoTab('Registry');
    expect(
      await screen.findByText(/No directory identities are currently listed/)
    ).toBeInTheDocument();
    expect(apiClient.directoryIdentities.list).toHaveBeenCalledTimes(1);
  });
});

// ── Registry tab ───────────────────────────────────────────────────────────────

describe('Registry tab', () => {
  test('shows the loading state while directory identities load', async () => {
    let resolve!: (v: { identities: never[] }) => void;
    vi.mocked(apiClient.directoryIdentities.list).mockReturnValue(
      new Promise(r => {
        resolve = r;
      })
    );
    render(<IdentitiesSection />);
    await gotoTab('Registry');
    expect(screen.getByText('Loading identities…')).toBeInTheDocument();
    resolve({ identities: [] });
    expect(
      await screen.findByText(/No directory identities are currently listed/i)
    ).toBeInTheDocument();
  });

  test('shows the empty state when no directory identities are listed', async () => {
    vi.mocked(apiClient.directoryIdentities.list).mockResolvedValue({ identities: [] });
    render(<IdentitiesSection />);
    await gotoTab('Registry');
    expect(
      await screen.findByText(/No directory identities are currently listed/i)
    ).toBeInTheDocument();
  });

  test('shows the empty state when identities field is missing entirely', async () => {
    // identities is undefined → nullish-coalesce to [] path
    vi.mocked(apiClient.directoryIdentities.list).mockResolvedValue(
      {} as unknown as { identities: never[] }
    );
    render(<IdentitiesSection />);
    await gotoTab('Registry');
    expect(
      await screen.findByText(/No directory identities are currently listed/i)
    ).toBeInTheDocument();
  });

  test('renders a populated table of directory identities with prices, dates and statuses', async () => {
    vi.mocked(apiClient.directoryIdentities.list).mockResolvedValue({
      identities: [
        {
          listingId: 'listing-1',
          name: '@alpha',
          seller: 'seller-one',
          updatedAt: '2026-02-03T00:00:00Z',
          status: 'active',
          price: { amount: '100', asset: 'USDC' },
        },
        // no seller, no price → em-dash fallbacks; non-active status branch
        { listingId: 'listing-2', name: '@beta', updatedAt: 'not-a-date', status: 'pending' },
      ] as unknown as DirectoryIdentityListingsResponse['identities'],
    });
    render(<IdentitiesSection />);
    await gotoTab('Registry');

    expect(await screen.findByText('@alpha')).toBeInTheDocument();
    expect(screen.getByText('seller-one')).toBeInTheDocument();
    expect(screen.getByText('100 USDC')).toBeInTheDocument();
    expect(screen.getByText('active')).toBeInTheDocument();

    expect(screen.getByText('@beta')).toBeInTheDocument();
    expect(screen.getByText('pending')).toBeInTheDocument();
    // invalid date falls back to the raw value
    expect(screen.getByText('not-a-date')).toBeInTheDocument();
    // missing seller + missing price render em-dashes
    expect(screen.getAllByText('—').length).toBeGreaterThanOrEqual(2);
  });

  test('renders status fallback of "unknown" when status is absent', async () => {
    vi.mocked(apiClient.directoryIdentities.list).mockResolvedValue({
      identities: [
        {
          listingId: 'listing-x',
          name: '@gamma',
          updatedAt: '2026-02-03T00:00:00Z',
          price: { amount: '5', asset: 'USDC' },
        },
      ],
    });
    render(<IdentitiesSection />);
    await gotoTab('Registry');
    expect(await screen.findByText('unknown')).toBeInTheDocument();
  });

  test('shows the error banner when the directory fetch rejects', async () => {
    vi.mocked(apiClient.directoryIdentities.list).mockRejectedValueOnce(
      new Error('directory down')
    );
    render(<IdentitiesSection />);
    await gotoTab('Registry');
    expect(await screen.findByText('Failed to load')).toBeInTheDocument();
    expect(screen.getByText(/directory down/)).toBeInTheDocument();
  });

  test('shows the wallet-locked banner when the error mentions a missing wallet', async () => {
    vi.mocked(apiClient.directoryIdentities.list).mockRejectedValueOnce(
      new Error('wallet is not configured')
    );
    render(<IdentitiesSection />);
    await gotoTab('Registry');
    expect(await screen.findByText('Unlock your wallet to use Agent World')).toBeInTheDocument();
  });

  test('shows the wallet-locked banner for missing secret material', async () => {
    vi.mocked(apiClient.directoryIdentities.list).mockRejectedValueOnce(
      new Error('wallet secret material is missing')
    );
    render(<IdentitiesSection />);
    await gotoTab('Registry');
    expect(await screen.findByText('Unlock your wallet to use Agent World')).toBeInTheDocument();
  });

  test('shows the payment-required banner when the directory fetch is gated', async () => {
    vi.mocked(apiClient.directoryIdentities.list).mockRejectedValueOnce(
      new PaymentRequiredError({ terms: 'x402' })
    );
    render(<IdentitiesSection />);
    await gotoTab('Registry');
    expect(await screen.findByText('Access requires payment')).toBeInTheDocument();
  });
});

// ── Trading tab — floor prices ─────────────────────────────────────────────────

describe('Trading tab — floor prices', () => {
  test('renders three floor cards (3 / 4 / 5+ chars) with their labels', async () => {
    render(<IdentitiesSection />);
    await gotoTab('Trading');
    expect(await screen.findByText('3 chars')).toBeInTheDocument();
    expect(screen.getByText('4 chars')).toBeInTheDocument();
    expect(screen.getByText('5+ chars')).toBeInTheDocument();
    expect(screen.getByText('Short handles')).toBeInTheDocument();
    expect(screen.getByText('Compact handles')).toBeInTheDocument();
    expect(screen.getByText('Long-form identities')).toBeInTheDocument();
  });

  test('shows a price when a floor card resolves with a price', async () => {
    vi.mocked(apiClient.marketplace.identityFloor).mockImplementation((length?: number) => {
      if (length === 3)
        return Promise.resolve({ length: 3, price: { amount: '250', asset: 'USDC' } });
      return Promise.resolve({ length, price: undefined });
    });
    render(<IdentitiesSection />);
    await gotoTab('Trading');
    expect(await screen.findByText('250 USDC')).toBeInTheDocument();
    // the other two cards resolve without a price → "No floor"
    expect(screen.getAllByText('No floor').length).toBeGreaterThanOrEqual(2);
    expect(apiClient.graphql.identityListings).toHaveBeenCalledWith({
      length: 3,
      limit: 20,
      offset: 0,
      sortBy: 'price_asc',
    });
  });

  test('ignores inactive cheaper listings when choosing a floor price', async () => {
    vi.mocked(apiClient.graphql.identityListings).mockImplementation(params => {
      if (params?.length === 3) {
        return Promise.resolve({
          identities: [
            {
              listingId: 'sold-floor',
              name: '@soldfloor',
              price: { amount: '50', asset: 'USDC' },
              status: 'sold',
              updatedAt: '2026-02-03T00:00:00Z',
            },
            {
              listingId: 'active-floor',
              name: '@activefloor',
              price: { amount: '250', asset: 'USDC' },
              status: 'active',
              updatedAt: '2026-02-03T00:00:00Z',
            },
          ],
        });
      }
      if (typeof params?.length === 'number') return Promise.resolve({ identities: [] });
      return Promise.resolve({ identities: [] });
    });
    render(<IdentitiesSection />);
    await gotoTab('Trading');
    expect(await screen.findByText('250 USDC')).toBeInTheDocument();
    expect(screen.queryByText('50 USDC')).not.toBeInTheDocument();
  });

  test('paginates floor lookup past inactive rows before declaring no floor', async () => {
    vi.mocked(apiClient.graphql.identityListings).mockImplementation(params => {
      if (params?.length !== 3) return Promise.resolve({ identities: [] });
      if ((params.offset ?? 0) === 0) {
        return Promise.resolve({
          identities: Array.from({ length: 20 }, (_, index) => ({
            listingId: `sold-floor-${index}`,
            name: `@soldfloor${index}`,
            price: { amount: '50', asset: 'USDC' },
            status: 'sold',
            updatedAt: '2026-02-03T00:00:00Z',
          })),
        });
      }
      return Promise.resolve({
        identities: [
          {
            listingId: 'active-floor-page-2',
            name: '@activefloorpage2',
            price: { amount: '250', asset: 'USDC' },
            status: 'active',
            updatedAt: '2026-02-03T00:00:00Z',
          },
        ],
      });
    });
    render(<IdentitiesSection />);
    await gotoTab('Trading');
    expect(await screen.findByText('250 USDC')).toBeInTheDocument();
    expect(apiClient.graphql.identityListings).toHaveBeenCalledWith({
      length: 3,
      limit: 20,
      offset: 0,
      sortBy: 'price_asc',
    });
    expect(apiClient.graphql.identityListings).toHaveBeenCalledWith({
      length: 3,
      limit: 20,
      offset: 20,
      sortBy: 'price_asc',
    });
  });

  test('shows "Unavailable" when a floor card fetch rejects', async () => {
    vi.mocked(apiClient.marketplace.identityFloor).mockRejectedValue(new Error('floor down'));
    render(<IdentitiesSection />);
    await gotoTab('Trading');
    expect((await screen.findAllByText('Unavailable')).length).toBe(3);
  });
});

// ── Trading tab — listed for sale ──────────────────────────────────────────────

describe('Trading tab — listed for sale', () => {
  test('shows the loading state while listings load', async () => {
    let resolve!: (v: { identities: never[] }) => void;
    vi.mocked(apiClient.marketplace.listIdentities).mockReturnValue(
      new Promise(r => {
        resolve = r;
      })
    );
    render(<IdentitiesSection />);
    await gotoTab('Trading');
    expect(screen.getByText('Loading listings…')).toBeInTheDocument();
    resolve({ identities: [] });
    expect(await screen.findByText('No identities listed for sale')).toBeInTheDocument();
  });

  test('shows the empty state when no identities are listed for sale', async () => {
    vi.mocked(apiClient.marketplace.listIdentities).mockResolvedValue({ identities: [] });
    render(<IdentitiesSection />);
    await gotoTab('Trading');
    expect(await screen.findByText('No identities listed for sale')).toBeInTheDocument();
  });

  test('renders listing cards including an auction badge and seller line', async () => {
    vi.mocked(apiClient.graphql.identityListings).mockImplementation(params => {
      if (typeof params?.length === 'number') return Promise.resolve({ identities: [] });
      return Promise.resolve({
        identities: [
          {
            listingId: 'sale-1',
            name: '@forsale',
            price: { amount: '42', asset: 'USDC' },
            listingType: 'auction',
            seller: 'seller-x',
            updatedAt: '2026-02-03T00:00:00Z',
          },
          {
            listingId: 'sale-2',
            name: '@fixedone',
            price: { amount: '7', asset: 'USDC' },
            listingType: 'fixed',
            updatedAt: '2026-02-03T00:00:00Z',
          },
        ],
      });
    });
    render(<IdentitiesSection />);
    await gotoTab('Trading');

    expect(await screen.findByText('@forsale')).toBeInTheDocument();
    expect(screen.getByText('Auction')).toBeInTheDocument();
    expect(screen.getByText('42 USDC')).toBeInTheDocument();
    expect(screen.getByText('by seller-x')).toBeInTheDocument();

    // fixed listing: no auction badge, no seller line
    expect(screen.getByText('@fixedone')).toBeInTheDocument();
    expect(screen.getByText('7 USDC')).toBeInTheDocument();
  });

  test('filters inactive GraphQL marketplace listings before rendering cards', async () => {
    vi.mocked(apiClient.graphql.identityListings).mockImplementation(params => {
      if (typeof params?.length === 'number') return Promise.resolve({ identities: [] });
      return Promise.resolve({
        identities: [
          {
            listingId: 'active-1',
            name: '@activeone',
            price: { amount: '5', asset: 'USDC' },
            status: 'active',
            updatedAt: '2026-02-03T00:00:00Z',
          },
          {
            listingId: 'sold-1',
            name: '@soldone',
            price: { amount: '9', asset: 'USDC' },
            status: 'sold',
            updatedAt: '2026-02-03T00:00:00Z',
          },
        ],
      });
    });
    render(<IdentitiesSection />);
    await gotoTab('Trading');

    expect(await screen.findByText('@activeone')).toBeInTheDocument();
    expect(screen.queryByText('@soldone')).not.toBeInTheDocument();
  });

  test('paginates GraphQL marketplace listings until active rows are found', async () => {
    vi.mocked(apiClient.graphql.identityListings).mockImplementation(params => {
      if (typeof params?.length === 'number') return Promise.resolve({ identities: [] });
      if ((params?.offset ?? 0) === 0) {
        return Promise.resolve({
          identities: Array.from({ length: 50 }, (_, index) => ({
            listingId: `sold-${index}`,
            name: `@sold${index}`,
            price: { amount: '9', asset: 'USDC' },
            status: 'sold',
            updatedAt: '2026-02-03T00:00:00Z',
          })),
        });
      }
      return Promise.resolve({
        identities: [
          {
            listingId: 'active-page-2',
            name: '@activepage2',
            price: { amount: '11', asset: 'USDC' },
            status: 'active',
            updatedAt: '2026-02-03T00:00:00Z',
          },
        ],
      });
    });
    render(<IdentitiesSection />);
    await gotoTab('Trading');

    expect(await screen.findByText('@activepage2')).toBeInTheDocument();
    expect(apiClient.graphql.identityListings).toHaveBeenCalledWith({ limit: 50, offset: 0 });
    expect(apiClient.graphql.identityListings).toHaveBeenCalledWith({ limit: 50, offset: 50 });
  });

  test('shows the payment-required banner when listings are gated', async () => {
    vi.mocked(apiClient.graphql.identityListings).mockImplementation(params => {
      if (typeof params?.length === 'number') return Promise.resolve({ identities: [] });
      return Promise.reject(new PaymentRequiredError({ terms: 'x402' }));
    });
    render(<IdentitiesSection />);
    await gotoTab('Trading');
    expect(await screen.findByText('Access requires payment')).toBeInTheDocument();
  });

  test('shows the error banner when listings fetch rejects', async () => {
    vi.mocked(apiClient.graphql.identityListings).mockImplementation(params => {
      if (typeof params?.length === 'number') return Promise.resolve({ identities: [] });
      return Promise.reject(new Error('listings down'));
    });
    render(<IdentitiesSection />);
    await gotoTab('Trading');
    expect(await screen.findByText('Failed to load')).toBeInTheDocument();
    expect(screen.getByText(/listings down/)).toBeInTheDocument();
  });
});

// ── Trading tab — recent sales ─────────────────────────────────────────────────

describe('Trading tab — recent sales', () => {
  test('shows the loading state while recent sales load', async () => {
    let resolve!: (v: { sales: never[] }) => void;
    vi.mocked(apiClient.marketplace.recent).mockReturnValue(
      new Promise(r => {
        resolve = r;
      })
    );
    render(<IdentitiesSection />);
    await gotoTab('Trading');
    expect(screen.getByText('Loading sales…')).toBeInTheDocument();
    resolve({ sales: [] });
    expect(await screen.findByText('No recent sales')).toBeInTheDocument();
  });

  test('shows the empty state when there are no recent sales', async () => {
    vi.mocked(apiClient.marketplace.recent).mockResolvedValue({ sales: [] });
    render(<IdentitiesSection />);
    await gotoTab('Trading');
    expect(await screen.findByText('No recent sales')).toBeInTheDocument();
  });

  test('renders a populated table of recent sales with truncated buyer + date', async () => {
    vi.mocked(apiClient.marketplace.recent).mockResolvedValue({
      sales: [
        {
          saleId: 'sale-a',
          name: '@solddomain',
          price: { amount: '999', asset: 'USDC' },
          buyer: 'buyer0123456789abcdef',
          createdAt: '2026-03-15T12:00:00Z',
        },
      ],
    });
    render(<IdentitiesSection />);
    await gotoTab('Trading');

    expect(await screen.findByText('@solddomain')).toBeInTheDocument();
    expect(screen.getByText('999 USDC')).toBeInTheDocument();
    // buyer truncated to first 12 chars + ellipsis
    expect(screen.getByText('buyer0123456...')).toBeInTheDocument();
    // createdAt sliced to YYYY-MM-DD
    expect(screen.getByText('2026-03-15')).toBeInTheDocument();
  });

  test('shows the sales error message when the recent sales fetch rejects', async () => {
    vi.mocked(apiClient.marketplace.recent).mockRejectedValueOnce(new Error('sales down'));
    render(<IdentitiesSection />);
    await gotoTab('Trading');
    expect(await screen.findByText('Failed to load sales')).toBeInTheDocument();
  });

  test('renders all three Trading sub-views together when populated', async () => {
    vi.mocked(apiClient.marketplace.identityFloor).mockImplementation((length?: number) =>
      length === 3
        ? Promise.resolve({ length: 3, price: { amount: '250', asset: 'USDC' } })
        : Promise.resolve({ length, price: undefined })
    );
    vi.mocked(apiClient.marketplace.listIdentities).mockResolvedValue({
      identities: [
        {
          listingId: 'sale-1',
          name: '@listed',
          price: { amount: '42', asset: 'USDC' },
          listingType: 'fixed',
          updatedAt: '2026-02-03T00:00:00Z',
        },
      ],
    });
    vi.mocked(apiClient.marketplace.recent).mockResolvedValue({
      sales: [
        {
          saleId: 'sale-a',
          name: '@sold',
          price: { amount: '999', asset: 'USDC' },
          buyer: 'buyerabcdef0123456789',
          createdAt: '2026-03-15T12:00:00Z',
        },
      ],
    });
    render(<IdentitiesSection />);
    await gotoTab('Trading');

    const floorSection = screen.getByText('Floor Prices').parentElement as HTMLElement;
    expect(within(floorSection).getByText('250 USDC')).toBeInTheDocument();
    expect(await screen.findByText('@listed')).toBeInTheDocument();
    expect(screen.getByText('@sold')).toBeInTheDocument();
  });
});

// ── Trading tab — buy identity (x402) ──────────────────────────────────────────

const fixedListing = {
  listingId: 'list-1',
  name: '@forsale',
  seller: 'seller-x',
  price: { amount: '20000000', asset: 'USDC' },
  listingType: 'fixed' as const,
  status: 'active',
  updatedAt: '2026-03-01T00:00:00Z',
};

describe('Trading tab — buy identity (x402)', () => {
  beforeEach(() => {
    vi.mocked(apiClient.marketplace.listIdentities).mockResolvedValue({
      identities: [fixedListing],
    });
  });

  test('Buy on a fixed-price listing opens the confirm dialog', async () => {
    vi.mocked(apiClient.marketplace.buyIdentity).mockResolvedValueOnce({
      challenge: { amount: '20000000', asset: 'USDC', network: 'solana-devnet' },
      walletBalance: { raw: '50000000', formatted: '50', decimals: 6, assetSymbol: 'USDC' },
      walletAddress: 'WalletXyz12345678',
    });
    render(<IdentitiesSection />);
    await gotoTab('Trading');
    await userEvent.click(await screen.findByRole('button', { name: 'Buy' }));

    expect(await screen.findByTestId('x402-amount')).toHaveTextContent('20 USDC');
    expect(apiClient.marketplace.buyIdentity).toHaveBeenCalledWith('list-1', { confirmed: false });
  });

  test('Confirm & Pay renders the purchased banner', async () => {
    vi.mocked(apiClient.marketplace.buyIdentity)
      .mockResolvedValueOnce({
        challenge: { amount: '20000000', asset: 'USDC', network: 'solana-devnet' },
        walletBalance: { raw: '50000000', formatted: '50', decimals: 6, assetSymbol: 'USDC' },
        walletAddress: 'WalletXyz12345678',
      })
      .mockResolvedValueOnce({ result: { saleId: 's1' }, payment: { onChainTx: 'TxId1' } });
    render(<IdentitiesSection />);
    await gotoTab('Trading');
    await userEvent.click(await screen.findByRole('button', { name: 'Buy' }));
    await userEvent.click(await screen.findByTestId('x402-confirm'));

    const success = await screen.findByTestId('buy-identity-success');
    expect(success).toHaveTextContent('Purchased @forsale');
    expect(apiClient.marketplace.buyIdentity).toHaveBeenLastCalledWith('list-1', {
      confirmed: true,
    });
  });

  test('auction listings do not show a Buy button', async () => {
    vi.mocked(apiClient.marketplace.listIdentities).mockResolvedValue({
      identities: [{ ...fixedListing, listingType: 'auction' as const }],
    });
    render(<IdentitiesSection />);
    await gotoTab('Trading');
    await screen.findByText('@forsale');
    expect(screen.queryByRole('button', { name: 'Buy' })).not.toBeInTheDocument();
  });

  test('Cancel closes the dialog without a confirmed buy', async () => {
    vi.mocked(apiClient.marketplace.buyIdentity).mockResolvedValueOnce({
      challenge: { amount: '20000000', asset: 'USDC', network: 'solana-devnet' },
      walletBalance: { raw: '50000000', formatted: '50', decimals: 6, assetSymbol: 'USDC' },
      walletAddress: 'WalletXyz12345678',
    });
    render(<IdentitiesSection />);
    await gotoTab('Trading');
    await userEvent.click(await screen.findByRole('button', { name: 'Buy' }));
    await userEvent.click(await screen.findByRole('button', { name: 'Cancel' }));
    expect(screen.queryByTestId('x402-confirm')).not.toBeInTheDocument();
    expect(apiClient.marketplace.buyIdentity).toHaveBeenCalledTimes(1);
  });
});

// ── Trading tab — bid / offer commitments (x402) ──────────────────────────────

const auctionListing = {
  listingId: 'auc-1',
  name: '@auction',
  seller: 'seller-y',
  price: { amount: '30000000', asset: 'USDC', network: 'solana-devnet' },
  listingType: 'auction' as const,
  status: 'active',
  updatedAt: '2026-03-02T00:00:00Z',
};

describe('Trading tab — bid / offer commitments', () => {
  test('Bid opens the amount dialog and submits a commitment', async () => {
    vi.mocked(apiClient.marketplace.listIdentities).mockResolvedValue({
      identities: [auctionListing],
    });
    render(<IdentitiesSection />);
    await gotoTab('Trading');
    await userEvent.click(await screen.findByRole('button', { name: 'Bid' }));

    await userEvent.type(screen.getByTestId('commit-amount-input'), '35000000');
    await userEvent.click(screen.getByTestId('commit-submit'));

    await screen.findByTestId('commit-success');
    expect(apiClient.marketplace.bid).toHaveBeenCalledWith('auc-1', {
      amount: '35000000',
      asset: 'USDC',
      network: 'solana-devnet',
    });
  });

  test('Offer submits a commitment for the handle', async () => {
    vi.mocked(apiClient.marketplace.listIdentities).mockResolvedValue({
      identities: [auctionListing],
    });
    render(<IdentitiesSection />);
    await gotoTab('Trading');
    await userEvent.click(await screen.findByRole('button', { name: 'Offer' }));

    await userEvent.type(screen.getByTestId('commit-amount-input'), '25000000');
    await userEvent.click(screen.getByTestId('commit-submit'));

    await screen.findByTestId('commit-success');
    expect(apiClient.marketplace.offer).toHaveBeenCalledWith('@auction', {
      amount: '25000000',
      asset: 'USDC',
      network: 'solana-devnet',
    });
  });

  test('a failed commitment surfaces an error banner', async () => {
    vi.mocked(apiClient.marketplace.listIdentities).mockResolvedValue({
      identities: [auctionListing],
    });
    vi.mocked(apiClient.marketplace.bid).mockRejectedValueOnce(new Error('bid-rejected'));
    render(<IdentitiesSection />);
    await gotoTab('Trading');
    await userEvent.click(await screen.findByRole('button', { name: 'Bid' }));
    await userEvent.type(screen.getByTestId('commit-amount-input'), '1');
    await userEvent.click(screen.getByTestId('commit-submit'));

    expect(await screen.findByTestId('commit-error')).toHaveTextContent('bid-rejected');
  });

  test('Cancel closes the commitment dialog without calling the API', async () => {
    vi.mocked(apiClient.marketplace.listIdentities).mockResolvedValue({
      identities: [auctionListing],
    });
    render(<IdentitiesSection />);
    await gotoTab('Trading');
    await userEvent.click(await screen.findByRole('button', { name: 'Offer' }));
    await userEvent.click(await screen.findByRole('button', { name: 'Cancel' }));
    expect(screen.queryByTestId('commit-submit')).not.toBeInTheDocument();
    expect(apiClient.marketplace.offer).not.toHaveBeenCalled();
  });
});
