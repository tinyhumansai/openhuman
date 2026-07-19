/**
 * Tests for ProfileViewer — the Agent World public profile viewer (#4931).
 *
 * The viewer renders an ARBITRARY handle's profile via `graphql.profile`, with a
 * follow/unfollow button (`follows.follow`/`unfollow`, follow-state from
 * `follows.following`) and a copy-link affordance. apiClient + wallet are mocked;
 * all handles/ids are generic placeholders. These behaviours do not exist before
 * this change (the route + component are new), so the file is the regression.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { type GqlProfile } from '../../lib/agentworld/invokeApiClient';
import { fetchWalletStatus } from '../../services/walletApi';
import { apiClient } from '../AgentWorldShell';
import ProfileViewer from './ProfileViewer';

vi.mock('../AgentWorldShell', () => ({
  apiClient: {
    graphql: { profile: vi.fn(), agentCard: vi.fn() },
    follows: { follow: vi.fn(), unfollow: vi.fn(), stats: vi.fn() },
  },
}));
vi.mock('../../services/walletApi', () => ({ fetchWalletStatus: vi.fn() }));

const graphqlProfile = vi.mocked(apiClient.graphql.profile);
const graphqlAgentCard = vi.mocked(apiClient.graphql.agentCard);
const followsFollow = vi.mocked(apiClient.follows.follow);
const followsUnfollow = vi.mocked(apiClient.follows.unfollow);
const followsStats = vi.mocked(apiClient.follows.stats);
const walletStatus = vi.mocked(fetchWalletStatus);

/** Agent card carrying the signer-aware follow flag the viewer reads. */
function agentCardFollowing(viewerIsFollowing: boolean) {
  return { agentId: PROFILE_ADDR, viewerIsFollowing };
}

const PROFILE_ADDR = 'ProfiLeSoLanaAddr00000000001';
const VIEWER_ADDR = 'ViewerSoLanaAddr00000000002';

function makeProfile(overrides: Partial<GqlProfile> = {}): GqlProfile {
  return {
    cryptoId: PROFILE_ADDR,
    actorType: 'agent',
    displayName: 'Alice Agent',
    bio: 'An autonomous test agent.',
    private: false,
    createdAt: '2026-01-02T00:00:00Z',
    updatedAt: '2026-01-02T00:00:00Z',
    verified: true,
    attestations: [],
    agentCard: null,
    identities: [
      {
        username: 'alice',
        cryptoId: PROFILE_ADDR,
        publicKey: 'pk',
        registeredAt: '2026-01-02T00:00:00Z',
        expiresAt: '2027-01-02T00:00:00Z',
        status: 'active',
        updatedAt: '2026-01-02T00:00:00Z',
        primary: true,
      },
    ],
    ...overrides,
  };
}

function walletWith(address: string | null) {
  return { accounts: address ? [{ chain: 'solana', address }] : [] } as unknown as Awaited<
    ReturnType<typeof fetchWalletStatus>
  >;
}

function renderViewer(username = 'alice') {
  return render(
    <MemoryRouter initialEntries={[`/agent-world/profiles/${username}`]}>
      <Routes>
        <Route path="/agent-world/profiles/:username" element={<ProfileViewer />} />
      </Routes>
    </MemoryRouter>
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  graphqlProfile.mockResolvedValue(makeProfile());
  // Default: viewer does not follow this agent yet.
  graphqlAgentCard.mockResolvedValue(agentCardFollowing(false));
  followsFollow.mockResolvedValue({ follower: VIEWER_ADDR, followee: PROFILE_ADDR, createdAt: '' });
  followsUnfollow.mockResolvedValue(undefined);
  followsStats.mockResolvedValue({ agentId: PROFILE_ADDR, followerCount: 3, followingCount: 5 });
  walletStatus.mockResolvedValue(walletWith(VIEWER_ADDR));
});

describe('ProfileViewer', () => {
  test('renders an arbitrary handle profile (not the wallet owner)', async () => {
    renderViewer('alice');
    // Looked up by the route param, not the wallet.
    await waitFor(() => expect(graphqlProfile).toHaveBeenCalledWith('alice'));
    // '@alice' shows in the header and again in the owned-handles list; assert
    // it renders at all (not a specific count) so the query stays unambiguous.
    expect((await screen.findAllByText('@alice')).length).toBeGreaterThan(0);
    expect(screen.getByText('An autonomous test agent.')).toBeInTheDocument();
    // Follower stats load in their own effect (a second async tick after the
    // profile), so await rather than assert synchronously.
    expect(await screen.findByText('3')).toBeInTheDocument();
  });

  test('shows a not-found state when the profile does not exist', async () => {
    graphqlProfile.mockResolvedValue(null);
    renderViewer('ghost');
    expect(await screen.findByText(/profile not found/i)).toBeInTheDocument();
  });

  test('follow button follows then unfollows another user', async () => {
    const user = userEvent.setup();
    renderViewer('alice');

    // Button appears once the wallet resolves and follow-state loads.
    const followBtn = await screen.findByRole('button', { name: 'Follow' });
    await waitFor(() => expect(followBtn).toBeEnabled());

    // Follower count starts at 3 (from follows.stats).
    expect(await screen.findByText('3')).toBeInTheDocument();

    await user.click(followBtn);
    expect(followsFollow).toHaveBeenCalledWith(PROFILE_ADDR);
    const followingBtn = await screen.findByRole('button', { name: 'Following' });
    // Count tracks the follow optimistically: 3 -> 4.
    expect(await screen.findByText('4')).toBeInTheDocument();

    await user.click(followingBtn);
    expect(followsUnfollow).toHaveBeenCalledWith(PROFILE_ADDR);
    expect(await screen.findByRole('button', { name: 'Follow' })).toBeInTheDocument();
    // ...and back to 3 on unfollow.
    expect(await screen.findByText('3')).toBeInTheDocument();
  });

  test('pre-selects the following state from the agent card follow flag', async () => {
    graphqlAgentCard.mockResolvedValue(agentCardFollowing(true));
    renderViewer('alice');
    // viewerIsFollowing:true → button shows Following without a click.
    expect(await screen.findByRole('button', { name: 'Following' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Follow' })).not.toBeInTheDocument();
  });

  test('hides the follow button when the follow relationship is unknown', async () => {
    // No agent card (e.g. a non-agent profile) → no follow flag → the button is
    // hidden rather than defaulting to an inferred state.
    graphqlAgentCard.mockResolvedValue(null);
    renderViewer('alice');
    await screen.findByTestId('profile-copy-link');
    expect(screen.queryByRole('button', { name: 'Follow' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Following' })).not.toBeInTheDocument();
  });

  test('hides the follow button and marks self when viewing own profile', async () => {
    walletStatus.mockResolvedValue(walletWith(PROFILE_ADDR));
    renderViewer('alice');
    expect(await screen.findByText(/this is your profile/i)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Follow' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Following' })).not.toBeInTheDocument();
  });

  test('copy-link affordance copies the shareable deep link', async () => {
    // Install a clipboard spy and drive the click with fireEvent — NOT
    // userEvent, whose setup() replaces navigator.clipboard with its own stub
    // and would shadow this spy.
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });

    renderViewer('alice');
    const copyBtn = await screen.findByTestId('profile-copy-link');
    fireEvent.click(copyBtn);

    await waitFor(() => expect(writeText).toHaveBeenCalled());
    expect(writeText.mock.calls[0][0] as string).toContain('#/agent-world/profiles/alice');
  });
});
