/**
 * Tests for ProfilesSection — Agent World "your profile" card.
 *
 * The page resolves the wallet's Solana address (`fetchWalletStatus`), reverse-
 * looks-up the handles registered to it (`apiClient.directory.reverse`), and
 * renders one of: loading / wallet_locked / no_handle / payment_required /
 * error / populated card. All handles/ids are GENERIC placeholders.
 */
import { render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { type GqlProfile, PaymentRequiredError } from '../../lib/agentworld/invokeApiClient';
import { fetchWalletStatus } from '../../services/walletApi';
import { apiClient } from '../AgentWorldShell';
import ProfilesSection from './ProfilesSection';

// ── Mocks ───────────────────────────────────────────────────────────────────
vi.mock('../AgentWorldShell', () => ({
  apiClient: {
    directory: { reverse: vi.fn() },
    follows: { stats: vi.fn() },
    registry: { export: vi.fn() },
    graphql: { user: vi.fn() },
  },
}));
vi.mock('../../services/walletApi', () => ({ fetchWalletStatus: vi.fn() }));

const reverse = vi.mocked(apiClient.directory.reverse);
const walletStatus = vi.mocked(fetchWalletStatus);
const followStats = vi.mocked(apiClient.follows.stats);
const registryExport = vi.mocked(apiClient.registry.export);
const graphqlUser = vi.mocked(apiClient.graphql.user);

const SOLANA_ADDR = 'WaLLetSoLanaAddr0123456789';

function walletWithSolana() {
  return {
    accounts: [
      { chain: 'evm', address: '0xabc', derivationPath: "m/44'/60'/0'/0/0" },
      { chain: 'solana', address: SOLANA_ADDR, derivationPath: "m/44'/501'/0'/0'" },
    ],
  } as unknown as Awaited<ReturnType<typeof fetchWalletStatus>>;
}

beforeEach(() => {
  vi.clearAllMocks();
  walletStatus.mockResolvedValue(walletWithSolana());
  // Default: graphql.user returns null so all existing tests exercise the directory.reverse
  // fallback path, which is identical to pre-Phase-4 behavior.
  graphqlUser.mockResolvedValue(null);
  reverse.mockResolvedValue({ cryptoId: SOLANA_ADDR, identities: [] });
  followStats.mockResolvedValue({ agentId: '', followerCount: 0, followingCount: 0 });
});

// ── Loading ───────────────────────────────────────────────────────────────────
describe('loading state', () => {
  test('shows the loading placeholder before resolution', () => {
    walletStatus.mockReturnValue(new Promise(() => {}));
    render(<ProfilesSection />);
    expect(screen.getByText(/Loading your profile…/i)).toBeInTheDocument();
  });
});

// ── Wallet locked ───────────────────────────────────────────────────────────────
describe('wallet_locked state', () => {
  test('prompts to unlock when wallet_status rejects', async () => {
    walletStatus.mockRejectedValueOnce(new Error('the wallet is not configured'));
    render(<ProfilesSection />);
    expect(await screen.findByText(/Unlock your wallet to use Agent World/i)).toBeInTheDocument();
    expect(reverse).not.toHaveBeenCalled();
  });

  test('prompts to unlock when there is no solana account', async () => {
    walletStatus.mockResolvedValueOnce({
      accounts: [{ chain: 'evm', address: '0xabc' }],
    } as unknown as Awaited<ReturnType<typeof fetchWalletStatus>>);
    render(<ProfilesSection />);
    expect(await screen.findByText(/Unlock your wallet to use Agent World/i)).toBeInTheDocument();
  });
});

// ── No handle ───────────────────────────────────────────────────────────────────
describe('no_handle state', () => {
  test('prompts to register when the wallet owns no handle', async () => {
    // graphqlUser returns null (default) so hook falls through to directory.reverse.
    reverse.mockResolvedValueOnce({ cryptoId: SOLANA_ADDR, identities: [] });
    render(<ProfilesSection />);
    expect(await screen.findByText(/No handle registered yet/i)).toBeInTheDocument();
    // Mentions the truncated wallet + points at the Identities tab.
    expect(screen.getByText(/Register one in the Identities tab/i)).toBeInTheDocument();
    // graphql.user was tried first before falling back.
    expect(graphqlUser).toHaveBeenCalledWith(SOLANA_ADDR);
    expect(reverse).toHaveBeenCalledWith(SOLANA_ADDR);
  });
});

// ── Payment required / error ───────────────────────────────────────────────────
describe('payment_required + error', () => {
  test('renders the x402 payment message', async () => {
    reverse.mockRejectedValueOnce(new PaymentRequiredError({ terms: 'x402' }));
    render(<ProfilesSection />);
    expect(await screen.findByText(/Access requires payment/i)).toBeInTheDocument();
  });

  test('renders a generic error for an unknown failure', async () => {
    reverse.mockRejectedValueOnce(new Error('boom: backend exploded'));
    render(<ProfilesSection />);
    expect(await screen.findByText(/Failed to load profile/i)).toBeInTheDocument();
    expect(screen.getByText(/boom: backend exploded/i)).toBeInTheDocument();
  });
});

// ── Populated card (the wallet's own handle) ────────────────────────────────────
describe('populated profile card', () => {
  test('renders the owned handle, cryptoId, and registration date', async () => {
    reverse.mockResolvedValueOnce({
      cryptoId: SOLANA_ADDR,
      identities: [
        {
          username: '@myhandle',
          cryptoId: SOLANA_ADDR,
          registeredAt: '2026-06-17T10:56:45.909Z',
          primary: true,
          status: 'active',
        },
      ],
    });
    render(<ProfilesSection />);
    expect(await screen.findByText('@myhandle')).toBeInTheDocument();
    // Truncated cryptoId (len > 12 → first6…last4).
    expect(screen.getByText('WaLLet…6789')).toBeInTheDocument();
    expect(screen.getByText(/Joined Jun 17, 2026/i)).toBeInTheDocument();
    // A bare handle has no published bio/skills.
    expect(screen.queryByText('Skills')).not.toBeInTheDocument();
  });

  test('picks the primary handle when the wallet owns several', async () => {
    reverse.mockResolvedValueOnce({
      cryptoId: SOLANA_ADDR,
      identities: [
        { username: '@secondary', cryptoId: SOLANA_ADDR, primary: false },
        { username: '@primaryhandle', cryptoId: SOLANA_ADDR, primary: true },
      ],
    });
    render(<ProfilesSection />);
    expect(await screen.findByText('@primaryhandle')).toBeInTheDocument();
    expect(screen.queryByText('@secondary')).not.toBeInTheDocument();
  });

  test('falls back to the first handle when none is marked primary', async () => {
    reverse.mockResolvedValueOnce({
      cryptoId: SOLANA_ADDR,
      identities: [
        { username: '@firsthandle', cryptoId: SOLANA_ADDR },
        { username: '@otherhandle', cryptoId: SOLANA_ADDR },
      ],
    });
    render(<ProfilesSection />);
    expect(await screen.findByText('@firsthandle')).toBeInTheDocument();
  });

  test('renders follower and following counts from follow stats', async () => {
    reverse.mockResolvedValueOnce({
      cryptoId: SOLANA_ADDR,
      identities: [{ username: '@statsuser', cryptoId: SOLANA_ADDR, primary: true }],
    });
    followStats.mockResolvedValueOnce({
      agentId: SOLANA_ADDR,
      followerCount: 42,
      followingCount: 7,
    });
    render(<ProfilesSection />);
    expect(await screen.findByText('@statsuser')).toBeInTheDocument();
    expect(await screen.findByText('42')).toBeInTheDocument();
    expect(screen.getByText('followers')).toBeInTheDocument();
    expect(screen.getByText('7')).toBeInTheDocument();
    expect(screen.getByText('following')).toBeInTheDocument();
  });

  test('renders singular follower when count is 1', async () => {
    reverse.mockResolvedValueOnce({
      cryptoId: SOLANA_ADDR,
      identities: [{ username: '@singlefollower', cryptoId: SOLANA_ADDR, primary: true }],
    });
    followStats.mockResolvedValueOnce({
      agentId: SOLANA_ADDR,
      followerCount: 1,
      followingCount: 0,
    });
    render(<ProfilesSection />);
    expect(await screen.findByText('@singlefollower')).toBeInTheDocument();
    expect(await screen.findByText('1')).toBeInTheDocument();
    expect(screen.getByText('follower')).toBeInTheDocument();
  });

  test('hides follow stats when the API call fails', async () => {
    reverse.mockResolvedValueOnce({
      cryptoId: SOLANA_ADDR,
      identities: [{ username: '@nostats', cryptoId: SOLANA_ADDR, primary: true }],
    });
    followStats.mockRejectedValueOnce(new Error('stats unavailable'));
    render(<ProfilesSection />);
    expect(await screen.findByText('@nostats')).toBeInTheDocument();
    // No follower/following counts rendered.
    expect(screen.queryByText('followers')).not.toBeInTheDocument();
    expect(screen.queryByText('following')).not.toBeInTheDocument();
  });
});

// ── Export identity button ────────────────────────────────────────────────────
describe('export identity button', () => {
  const IDENTITY_EXPORT = {
    identity: {
      username: '@exportuser',
      cryptoId: SOLANA_ADDR,
      publicKey: 'pk-abc',
      registeredAt: '2025-01-01T00:00:00Z',
      expiresAt: '2026-01-01T00:00:00Z',
      status: 'ACTIVE',
      updatedAt: '2025-06-01T00:00:00Z',
    },
    ledgerTransactions: [],
    exportedAt: '2025-06-15T12:00:00Z',
    verification: { hash: 'abc123' },
    proofs: {
      ownership: {
        algorithm: 'ed25519',
        cryptoId: SOLANA_ADDR,
        publicKey: 'pk-abc',
        publicKeyMatchesCryptoId: true,
      },
      ledgerReferences: [],
    },
  };

  function renderWithHandle() {
    reverse.mockResolvedValueOnce({
      cryptoId: SOLANA_ADDR,
      identities: [{ username: '@exportuser', cryptoId: SOLANA_ADDR, primary: true }],
    });
    followStats.mockResolvedValueOnce({
      agentId: SOLANA_ADDR,
      followerCount: 0,
      followingCount: 0,
    });
  }

  test('renders Export Identity button on the profile card', async () => {
    renderWithHandle();
    render(<ProfilesSection />);
    expect(await screen.findByText('@exportuser')).toBeInTheDocument();
    expect(screen.getByText('Export Identity')).toBeInTheDocument();
  });

  test('clicking Export Identity fetches and displays the export JSON', async () => {
    const user = (await import('@testing-library/user-event')).default.setup();
    renderWithHandle();
    registryExport.mockResolvedValueOnce(IDENTITY_EXPORT);
    render(<ProfilesSection />);
    const btn = await screen.findByText('Export Identity');
    await user.click(btn);
    expect(registryExport).toHaveBeenCalledWith('@exportuser');
    // JSON is displayed in a <pre> block.
    expect(await screen.findByText(/exportedAt/)).toBeInTheDocument();
    // Button label changes to "Hide Export".
    expect(screen.getByText('Hide Export')).toBeInTheDocument();
  });

  test('clicking Hide Export clears the export panel', async () => {
    const user = (await import('@testing-library/user-event')).default.setup();
    renderWithHandle();
    registryExport.mockResolvedValueOnce(IDENTITY_EXPORT);
    render(<ProfilesSection />);
    const btn = await screen.findByText('Export Identity');
    await user.click(btn);
    expect(await screen.findByText('Hide Export')).toBeInTheDocument();
    await user.click(screen.getByText('Hide Export'));
    // Panel is hidden, button reverts.
    expect(screen.getByText('Export Identity')).toBeInTheDocument();
    expect(screen.queryByText(/exportedAt/)).not.toBeInTheDocument();
  });

  test('shows error message when export fails', async () => {
    const user = (await import('@testing-library/user-event')).default.setup();
    renderWithHandle();
    registryExport.mockRejectedValueOnce(new Error('Network error'));
    render(<ProfilesSection />);
    const btn = await screen.findByText('Export Identity');
    await user.click(btn);
    expect(await screen.findByText(/Network error/)).toBeInTheDocument();
    // Button still says "Export Identity" (not "Hide Export") since data is null.
    expect(screen.getByText('Export Identity')).toBeInTheDocument();
  });
});

// ── Cancellation ────────────────────────────────────────────────────────────────
describe('cancellation', () => {
  test('does not update state after unmount', async () => {
    let resolve!: (v: Awaited<ReturnType<typeof fetchWalletStatus>>) => void;
    walletStatus.mockReturnValue(
      new Promise(r => {
        resolve = r;
      })
    );
    const { unmount } = render(<ProfilesSection />);
    unmount();
    resolve(walletWithSolana());
    await waitFor(() => expect(walletStatus).toHaveBeenCalled());
  });
});

// ── GraphQL-enriched profile card ─────────────────────────────────────────────

/** Minimal identity fields needed to satisfy GqlProfile.identities[]. */
const minimalIdentity = {
  publicKey: 'pubkey-test',
  registeredAt: '2026-01-01T00:00:00Z',
  expiresAt: '2027-01-01T00:00:00Z',
  status: 'active',
  updatedAt: '2026-01-01T00:00:00Z',
};

/** Build a minimal GqlProfile for test mocks. */
function makeProfile(overrides: Partial<GqlProfile> = {}): GqlProfile {
  return {
    cryptoId: SOLANA_ADDR,
    actorType: 'agent',
    displayName: 'Test Agent',
    bio: '',
    private: false,
    createdAt: '2026-01-01T00:00:00Z',
    updatedAt: '2026-01-01T00:00:00Z',
    verified: false,
    attestations: [],
    agentCard: null,
    identities: null,
    ...overrides,
  };
}

describe('graphql-enriched profile card', () => {
  test('renders rich profile from graphql.user when available', async () => {
    graphqlUser.mockResolvedValueOnce(
      makeProfile({
        displayName: 'Agent Alice',
        bio: 'Building the future',
        tags: ['ai', 'automation'],
        verified: true,
        attestations: [
          {
            attestationId: 'att-1',
            platform: 'github',
            handle: 'alice',
            proofUrl: 'https://github.com/alice',
            status: 'verified',
            verifiedAt: '2026-02-01T00:00:00Z',
          },
        ],
        identities: [
          { username: 'alice', cryptoId: SOLANA_ADDR, primary: true, ...minimalIdentity },
        ],
      })
    );
    render(<ProfilesSection />);

    // Rich data rendered — the handle appears in the header heading (it also
    // appears in the "Handles owned" list, so target the heading specifically).
    expect(await screen.findByRole('heading', { name: /@alice/ })).toBeInTheDocument();
    expect(screen.getByText('Building the future')).toBeInTheDocument();
    expect(screen.getByText('ai')).toBeInTheDocument();
    expect(screen.getByText('automation')).toBeInTheDocument();
    // Attestation row
    expect(screen.getByText(/github.*alice/i)).toBeInTheDocument();
    // Verified Accounts section heading
    expect(screen.getByText('Verified Accounts')).toBeInTheDocument();
    // directory.reverse should NOT have been called (graphql.user succeeded)
    expect(reverse).not.toHaveBeenCalled();
  });

  test('falls back to directory.reverse when graphql.user returns null', async () => {
    graphqlUser.mockResolvedValueOnce(null);
    reverse.mockResolvedValueOnce({
      cryptoId: SOLANA_ADDR,
      identities: [{ username: '@fallbackuser', cryptoId: SOLANA_ADDR, primary: true }],
    });
    render(<ProfilesSection />);
    expect(await screen.findByText('@fallbackuser')).toBeInTheDocument();
    expect(graphqlUser).toHaveBeenCalledWith(SOLANA_ADDR);
    expect(reverse).toHaveBeenCalledWith(SOLANA_ADDR);
  });

  test('falls back to directory.reverse when graphql.user throws non-402 error', async () => {
    graphqlUser.mockRejectedValueOnce(new Error('GraphQL endpoint unreachable'));
    reverse.mockResolvedValueOnce({
      cryptoId: SOLANA_ADDR,
      identities: [{ username: '@resilientuser', cryptoId: SOLANA_ADDR, primary: true }],
    });
    render(<ProfilesSection />);
    expect(await screen.findByText('@resilientuser')).toBeInTheDocument();
    expect(reverse).toHaveBeenCalledWith(SOLANA_ADDR);
  });

  test('does NOT fall back when graphql.user throws PaymentRequiredError', async () => {
    graphqlUser.mockRejectedValueOnce(new PaymentRequiredError({ terms: 'x402' }));
    render(<ProfilesSection />);
    expect(await screen.findByText(/Access requires payment/i)).toBeInTheDocument();
    expect(reverse).not.toHaveBeenCalled();
  });

  test('renders profile with null identities (profile exists but no registered handle)', async () => {
    graphqlUser.mockResolvedValueOnce(
      makeProfile({ displayName: 'No Handle Agent', bio: '', identities: null })
    );
    render(<ProfilesSection />);
    expect(await screen.findByText('No Handle Agent')).toBeInTheDocument();
    expect(reverse).not.toHaveBeenCalled();
  });

  test('renders profile with empty attestations array — no Verified Accounts section', async () => {
    graphqlUser.mockResolvedValueOnce(
      makeProfile({
        displayName: 'Plain Agent',
        bio: 'No attestations here',
        attestations: [],
        identities: [
          { username: 'plain', cryptoId: SOLANA_ADDR, primary: true, ...minimalIdentity },
        ],
      })
    );
    render(<ProfilesSection />);
    expect(await screen.findByText('No attestations here')).toBeInTheDocument();
    // "Verified Accounts" section should not render when attestations is empty
    expect(screen.queryByText('Verified Accounts')).not.toBeInTheDocument();
  });
});
