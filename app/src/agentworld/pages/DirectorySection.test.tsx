/**
 * Tests for DirectorySection — Agent World directory grid.
 *
 * The page loads the agent directory via `apiClient.graphql.agents()` and
 * renders one of: loading skeleton / payment_required / error (generic + wallet
 * locked) / empty / populated grid of agent cards. Each card derives a handle,
 * initials, avatar colour and skills/tags from the raw `AgentCard`, and toggles
 * a "selected" ring on click / Enter / Space. We mock the apiClient so no RPC
 * fires and the render stays deterministic.
 */
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { PaymentRequiredError } from '../../lib/agentworld/invokeApiClient';
import { fetchWalletStatus } from '../../services/walletApi';
import { apiClient } from '../AgentWorldShell';
import DirectorySection from './DirectorySection';

// ── Mock apiClient ────────────────────────────────────────────────────────────

vi.mock('../AgentWorldShell', () => ({
  apiClient: {
    graphql: { agents: vi.fn() },
    follows: {
      stats: vi.fn(),
      followers: vi.fn(),
      following: vi.fn(),
      follow: vi.fn(),
      unfollow: vi.fn(),
    },
  },
}));

vi.mock('../../services/walletApi', () => ({ fetchWalletStatus: vi.fn() }));

const listAgents = vi.mocked(apiClient.graphql.agents);
const walletStatus = vi.mocked(fetchWalletStatus);
const followFollow = vi.mocked(apiClient.follows.follow);
const followUnfollow = vi.mocked(apiClient.follows.unfollow);

beforeEach(() => {
  vi.clearAllMocks();
  // Default: wallet returns a Solana address.
  walletStatus.mockResolvedValue({
    accounts: [{ chain: 'solana', address: 'MyWaLLetAddr123' }],
  } as unknown as Awaited<ReturnType<typeof fetchWalletStatus>>);
  vi.mocked(apiClient.follows.stats).mockResolvedValue({
    agentId: 'fallback-agent',
    followerCount: 0,
    followingCount: 0,
  });
});

// ── Loading state ──────────────────────────────────────────────────────────────

describe('loading state', () => {
  test('renders the pulsing skeleton grid before the fetch settles', () => {
    // A promise that never resolves keeps the component in `loading`.
    listAgents.mockReturnValue(new Promise(() => {}));
    const { container } = render(<DirectorySection />);
    // The skeleton renders six animate-pulse placeholder cards.
    const skeletons = container.querySelectorAll('.animate-pulse');
    expect(skeletons).toHaveLength(6);
    // No status text or real cards yet.
    expect(screen.queryByText(/No agents found/i)).not.toBeInTheDocument();
  });
});

// ── Payment required state ──────────────────────────────────────────────────────

describe('payment_required state', () => {
  test('renders the x402 wallet payment message', async () => {
    listAgents.mockRejectedValueOnce(new PaymentRequiredError({ terms: 'x402' }));
    render(<DirectorySection />);
    expect(await screen.findByText(/Access requires payment/i)).toBeInTheDocument();
    expect(
      screen.getByText(/Your wallet will be used to fulfill the x402 payment challenge/i)
    ).toBeInTheDocument();
  });
});

// ── Error states ────────────────────────────────────────────────────────────────

describe('error state', () => {
  test('renders a generic error message for an unknown failure', async () => {
    listAgents.mockRejectedValueOnce(new Error('boom: backend exploded'));
    render(<DirectorySection />);
    expect(await screen.findByText(/Failed to load Directory/i)).toBeInTheDocument();
    // `String(err)` includes the "Error: " prefix.
    expect(screen.getByText(/boom: backend exploded/i)).toBeInTheDocument();
  });

  test('renders the wallet-unlock prompt when the wallet is not configured', async () => {
    listAgents.mockRejectedValueOnce(new Error('the wallet is not configured'));
    render(<DirectorySection />);
    expect(
      await screen.findByText(/Unlock your wallet to browse the Directory/i)
    ).toBeInTheDocument();
    expect(screen.getByText(/Import your recovery phrase in Settings/i)).toBeInTheDocument();
  });

  test('renders the wallet-unlock prompt when wallet secret material is missing', async () => {
    listAgents.mockRejectedValueOnce(new Error('wallet secret material is missing'));
    render(<DirectorySection />);
    expect(
      await screen.findByText(/Unlock your wallet to browse the Directory/i)
    ).toBeInTheDocument();
  });
});

// ── Empty state ─────────────────────────────────────────────────────────────────

describe('empty state', () => {
  test('renders "No agents found" when the directory returns no agents', async () => {
    listAgents.mockResolvedValueOnce({ agents: [] });
    render(<DirectorySection />);
    expect(await screen.findByText(/No agents found/i)).toBeInTheDocument();
    expect(screen.getByText(/No agents are registered in the directory yet/i)).toBeInTheDocument();
  });

  test('renders the empty state when the data has an explicit empty agents array', async () => {
    // The `?? []` render fallback guards against a null/undefined `agents` field
    // surviving into the ok state; an explicit [] takes the same empty branch.
    listAgents.mockResolvedValueOnce({ agents: [] } as { agents: [] });
    render(<DirectorySection />);
    expect(await screen.findByText(/No agents found/i)).toBeInTheDocument();
  });

  test('surfaces an error when the payload omits the agents array', async () => {
    // The success handler reads `data.agents.length`, so a missing `agents`
    // field throws and lands in the error branch rather than the empty state.
    listAgents.mockResolvedValueOnce({} as { agents: [] });
    render(<DirectorySection />);
    expect(await screen.findByText(/Failed to load Directory/i)).toBeInTheDocument();
  });
});

// ── Populated grid ──────────────────────────────────────────────────────────────

describe('populated directory grid', () => {
  test('renders a fully-populated agent card (handle, initials, description, skills)', async () => {
    listAgents.mockResolvedValueOnce({
      agents: [
        {
          agentId: 'agent-001',
          username: 'aurora',
          name: 'Aurora',
          description: 'A helpful demo agent.',
          skills: ['research', { id: 'sk-2', name: 'writing' }, { id: 'sk-3' }, 42],
        },
      ],
    });
    render(<DirectorySection />);

    // Handle (username already without a leading @ → @aurora).
    expect(await screen.findByText('@aurora')).toBeInTheDocument();
    // Description.
    expect(screen.getByText('A helpful demo agent.')).toBeInTheDocument();
    // Initials derived from display name (first two chars, uppercased).
    expect(screen.getByText('AU')).toBeInTheDocument();
    // Skills normalised: strings pass through, objects use `name` when present,
    // otherwise `String(s)` ('[object Object]' for nameless objects, '42' for numbers).
    expect(screen.getByText('research')).toBeInTheDocument();
    expect(screen.getByText('writing')).toBeInTheDocument();
    expect(screen.getByText('42')).toBeInTheDocument();
    // The `{ id: 'sk-3' }` object has no `name` → stringified to [object Object].
    expect(screen.getByText('[object Object]')).toBeInTheDocument();
  });

  test('strips a leading @ from username and falls back to tags for skills', async () => {
    listAgents.mockResolvedValueOnce({
      agents: [{ agentId: 'agent-002', username: '@nimbus', name: 'Nimbus', tags: ['ops'] }],
    });
    render(<DirectorySection />);
    // Leading @ stripped then re-added → single @nimbus.
    expect(await screen.findByText('@nimbus')).toBeInTheDocument();
    // Falls back to `tags` when `skills` is absent.
    expect(screen.getByText('ops')).toBeInTheDocument();
  });

  test('uses `name` for the handle when `username` is absent', async () => {
    listAgents.mockResolvedValueOnce({ agents: [{ agentId: 'agent-003', name: 'Solo' }] });
    render(<DirectorySection />);
    expect(await screen.findByText('@Solo')).toBeInTheDocument();
  });

  test('falls back to the first 8 chars of agentId when name and username are absent', async () => {
    listAgents.mockResolvedValueOnce({ agents: [{ agentId: 'abcdef0123456789' }] });
    render(<DirectorySection />);
    // displayName = agentId.slice(0, 8) → 'abcdef01'; handle = '@abcdef01'.
    expect(await screen.findByText('@abcdef01')).toBeInTheDocument();
    // Initials = first two chars uppercased → 'AB'.
    expect(screen.getByText('AB')).toBeInTheDocument();
  });

  test('renders an empty description and hides the skills row when no skills/tags', async () => {
    listAgents.mockResolvedValueOnce({
      agents: [{ agentId: 'agent-bare', username: 'bare', name: 'Bare' }],
    });
    const { container } = render(<DirectorySection />);
    expect(await screen.findByText('@bare')).toBeInTheDocument();
    // No skills pills are rendered.
    expect(container.querySelector('.rounded-full.bg-stone-100')).not.toBeInTheDocument();
  });

  test('renders one card per agent when several are returned', async () => {
    listAgents.mockResolvedValueOnce({
      agents: [
        { agentId: 'agent-005', username: 'first', name: 'First' },
        { agentId: 'agent-006', username: 'second', name: 'Second' },
      ],
    });
    render(<DirectorySection />);
    expect(await screen.findByText('@first')).toBeInTheDocument();
    expect(screen.getByText('@second')).toBeInTheDocument();
    // Each agent has a card (role=button) plus a Follow button once the wallet and
    // follow-state resolve. Use getAllByText to verify two cards rendered.
    expect(screen.getAllByText(/^@(first|second)$/)).toHaveLength(2);
  });
});

// ── Interactions ────────────────────────────────────────────────────────────────

describe('card selection', () => {
  test('toggles the selected ring on click', async () => {
    const user = userEvent.setup();
    listAgents.mockResolvedValueOnce({
      agents: [{ agentId: 'agent-007', username: 'clicky', name: 'Clicky' }],
    });
    render(<DirectorySection />);
    await screen.findByText('@clicky');

    // The card is a div[role=button]; the Follow button is a <button type=button>.
    // Select the outer card div by its data-testid class (cursor-pointer).
    const card = screen.getAllByRole('button').find(el => el.tagName === 'DIV') as HTMLElement;
    // Not selected initially.
    expect(card.className).not.toContain('ring-1');

    await user.click(card);
    expect(card.className).toContain('ring-1');

    // Clicking again deselects.
    await user.click(card);
    expect(card.className).not.toContain('ring-1');
  });

  test('toggles selection with the Enter key', async () => {
    const user = userEvent.setup();
    listAgents.mockResolvedValueOnce({
      agents: [{ agentId: 'agent-008', username: 'enterkey', name: 'EnterKey' }],
    });
    render(<DirectorySection />);
    await screen.findByText('@enterkey');

    const card = screen.getAllByRole('button').find(el => el.tagName === 'DIV') as HTMLElement;
    card.focus();
    await user.keyboard('{Enter}');
    expect(card.className).toContain('ring-1');
  });

  test('toggles selection with the Space key', async () => {
    const user = userEvent.setup();
    listAgents.mockResolvedValueOnce({
      agents: [{ agentId: 'agent-009', username: 'spacer', name: 'Spacer' }],
    });
    render(<DirectorySection />);
    await screen.findByText('@spacer');

    const card = screen.getAllByRole('button').find(el => el.tagName === 'DIV') as HTMLElement;
    card.focus();
    await user.keyboard('[Space]');
    expect(card.className).toContain('ring-1');
  });

  test('ignores other keys', async () => {
    const user = userEvent.setup();
    listAgents.mockResolvedValueOnce({
      agents: [{ agentId: 'agent-010', username: 'idle', name: 'Idle' }],
    });
    render(<DirectorySection />);
    await screen.findByText('@idle');

    const card = screen.getAllByRole('button').find(el => el.tagName === 'DIV') as HTMLElement;
    card.focus();
    await user.keyboard('{Escape}');
    expect(card.className).not.toContain('ring-1');
  });

  test('selecting one card leaves the other unaffected', async () => {
    const user = userEvent.setup();
    listAgents.mockResolvedValueOnce({
      agents: [
        { agentId: 'agent-011', username: 'alpha', name: 'Alpha' },
        { agentId: 'agent-012', username: 'beta', name: 'Beta' },
      ],
    });
    render(<DirectorySection />);
    await screen.findByText('@alpha');

    // Card divs have role=button; Follow <button>s also have role=button.
    // Filter for the outer card divs only.
    const cards = screen.getAllByRole('button').filter(el => el.tagName === 'DIV');
    const alphaCard = cards.find(c => within(c).queryByText('@alpha')) as HTMLElement;
    const betaCard = cards.find(c => within(c).queryByText('@beta')) as HTMLElement;

    await user.click(alphaCard);
    expect(alphaCard.className).toContain('ring-1');
    expect(betaCard.className).not.toContain('ring-1');
  });
});

// ── Cleanup / cancellation ──────────────────────────────────────────────────────

describe('cancellation', () => {
  test('does not update state after unmount (no act warning on late resolve)', async () => {
    let resolve!: (v: { agents: [] }) => void;
    listAgents.mockReturnValue(
      new Promise(r => {
        resolve = r;
      })
    );
    const { unmount } = render(<DirectorySection />);
    unmount();
    // Resolve after unmount — the cancelled flag should swallow the update.
    resolve({ agents: [] });
    await waitFor(() => expect(listAgents).toHaveBeenCalled());
  });
});

// ── Follow button ─────────────────────────────────────────────────────────────

describe('follow button', () => {
  test('renders Follow button on agent cards when wallet is available', async () => {
    listAgents.mockResolvedValueOnce({
      agents: [
        {
          agentId: 'other-agent-001',
          username: 'alice',
          name: 'Alice',
          viewerIsFollowing: false,
          followerCount: 5,
        },
      ],
    });
    render(<DirectorySection />);
    expect(await screen.findByText('@alice')).toBeInTheDocument();
    // Should render a Follow button since we are NOT following.
    expect(await screen.findByText('Follow')).toBeInTheDocument();
    // Should render follower count.
    expect(await screen.findByText(/5 followers/)).toBeInTheDocument();
  });

  test('renders Following button when already following', async () => {
    listAgents.mockResolvedValueOnce({
      agents: [
        {
          agentId: 'other-agent-002',
          username: 'bob',
          name: 'Bob',
          viewerIsFollowing: true,
          followerCount: 10,
        },
      ],
    });
    render(<DirectorySection />);
    expect(await screen.findByText('Following')).toBeInTheDocument();
  });

  test('does not render Follow button on own agent card', async () => {
    listAgents.mockResolvedValueOnce({
      agents: [{ agentId: 'MyWaLLetAddr123', username: 'myself', name: 'Myself' }],
    });
    render(<DirectorySection />);
    expect(await screen.findByText('@myself')).toBeInTheDocument();
    // No Follow button for self.
    expect(screen.queryByText('Follow')).not.toBeInTheDocument();
    expect(screen.queryByText('Following')).not.toBeInTheDocument();
  });

  test('clicking Follow calls follows.follow and updates to Following', async () => {
    const user = userEvent.setup();
    listAgents.mockResolvedValueOnce({
      agents: [
        {
          agentId: 'other-agent-003',
          username: 'carol',
          name: 'Carol',
          viewerIsFollowing: false,
          followerCount: 0,
        },
      ],
    });
    followFollow.mockResolvedValueOnce({
      follower: 'MyWaLLetAddr123',
      followee: 'other-agent-003',
      createdAt: '',
    });
    render(<DirectorySection />);
    const followBtn = await screen.findByText('Follow');
    await user.click(followBtn);
    expect(followFollow).toHaveBeenCalledWith('other-agent-003');
    expect(await screen.findByText('Following')).toBeInTheDocument();
  });

  test('uses GraphQL follow edges and count-only stats fallback without follow-list fan-out', async () => {
    vi.mocked(apiClient.follows.stats).mockImplementation(agentId =>
      Promise.resolve({
        agentId,
        followerCount: agentId === 'other-agent-b' ? 7 : 0,
        followingCount: 0,
      })
    );
    listAgents.mockResolvedValueOnce({
      agents: [
        { agentId: 'other-agent-a', username: 'a', name: 'A', viewerIsFollowing: false },
        { agentId: 'other-agent-b', username: 'b', name: 'B', viewerIsFollowing: true },
        { agentId: 'other-agent-c', username: 'c', name: 'C', viewerIsFollowing: false },
      ],
    });
    render(<DirectorySection />);

    expect(await screen.findByText('Following')).toBeInTheDocument();
    expect(await screen.findByText('7 followers')).toBeInTheDocument();
    expect(apiClient.follows.following).not.toHaveBeenCalled();
    expect(apiClient.follows.stats).toHaveBeenCalledTimes(3);
    expect(apiClient.follows.followers).not.toHaveBeenCalled();
  });

  test('clicking Following calls follows.unfollow and reverts to Follow', async () => {
    const user = userEvent.setup();
    listAgents.mockResolvedValueOnce({
      agents: [
        {
          agentId: 'other-agent-004',
          username: 'dave',
          name: 'Dave',
          viewerIsFollowing: true,
          followerCount: 1,
        },
      ],
    });
    followUnfollow.mockResolvedValueOnce(undefined);
    render(<DirectorySection />);
    const followingBtn = await screen.findByText('Following');
    await user.click(followingBtn);
    expect(followUnfollow).toHaveBeenCalledWith('other-agent-004');
    expect(await screen.findByText('Follow')).toBeInTheDocument();
  });
});
