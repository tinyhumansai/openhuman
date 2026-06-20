/**
 * Smoke test for `RewardsCommunityTab` — exercises the `role.unlocked`
 * branch (line 248) added by PR #2095's dark-mode pass so the diff
 * coverage gate has the touched line covered.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { RewardsSnapshot } from '../../../types/rewards';

const { openUrl, callCoreRpc, setOAuthReturnRoute, disconnectDiscord } = vi.hoisted(() => ({
  openUrl: vi.fn(),
  callCoreRpc: vi.fn(),
  setOAuthReturnRoute: vi.fn(),
  disconnectDiscord: vi.fn(),
}));

vi.mock('../../../utils/openUrl', () => ({ openUrl }));
vi.mock('../../../services/coreRpcClient', () => ({ callCoreRpc }));
vi.mock('../../../services/api/rewardsApi', () => ({ rewardsApi: { disconnectDiscord } }));
vi.mock('../../../utils/oauthReturnRoute', () => ({ setOAuthReturnRoute }));

function buildSnapshot(): RewardsSnapshot {
  return {
    discord: {
      linked: true,
      discordId: 'discord-1',
      username: 'cooluser',
      inviteUrl: 'https://discord.gg/example',
      membershipStatus: 'member',
    },
    summary: {
      unlockedCount: 1,
      totalCount: 2,
      assignedDiscordRoleCount: 1,
      plan: 'FREE',
      hasActiveSubscription: false,
    },
    metrics: {
      currentStreakDays: 3,
      longestStreakDays: 5,
      cumulativeTokens: 1234,
      featuresUsedCount: 2,
      trackedFeaturesCount: 5,
      lastEvaluatedAt: null,
      lastSyncedAt: null,
    },
    achievements: [
      {
        id: 'role-1',
        title: 'Pioneer',
        description: 'Joined early.',
        actionLabel: 'View',
        unlocked: true,
        progressLabel: '1/1',
        roleId: 'discord-role-1',
        discordRoleStatus: 'assigned',
        creditAmountUsd: null,
      },
      {
        id: 'role-2',
        title: 'Veteran',
        description: 'Long streak.',
        actionLabel: 'View',
        unlocked: false,
        progressLabel: '0/1',
        roleId: 'discord-role-2',
        discordRoleStatus: 'not_assigned',
        creditAmountUsd: null,
      },
    ],
  };
}

describe('RewardsCommunityTab — role card branches', () => {
  it('renders both unlocked and locked roles (covers the `role.unlocked` ring branch)', async () => {
    const { default: RewardsCommunityTab } = await import('../RewardsCommunityTab');
    render(
      <MemoryRouter>
        <RewardsCommunityTab error={null} isLoading={false} snapshot={buildSnapshot()} />
      </MemoryRouter>
    );

    // Both role titles are rendered — each goes through the ternary on
    // line 248 (ring-primary-100 for unlocked, ring-black/[0.04] for locked).
    expect(screen.getByText('Pioneer')).toBeInTheDocument();
    expect(screen.getByText('Veteran')).toBeInTheDocument();
  });
});

describe('RewardsCommunityTab — Connect Discord', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  function notLinkedSnapshot(): RewardsSnapshot {
    const snapshot = buildSnapshot();
    return {
      ...snapshot,
      discord: {
        linked: false,
        discordId: null,
        username: null,
        inviteUrl: 'https://discord.gg/example',
        membershipStatus: 'not_linked',
      },
    };
  }

  it('starts the OAuth flow and opens the consent URL on connect', async () => {
    callCoreRpc.mockResolvedValueOnce({ result: { oauthUrl: 'https://discord.com/oauth' } });
    const { default: RewardsCommunityTab } = await import('../RewardsCommunityTab');
    render(
      <MemoryRouter>
        <RewardsCommunityTab error={null} isLoading={false} snapshot={notLinkedSnapshot()} />
      </MemoryRouter>
    );

    fireEvent.click(screen.getByTestId('rewards-connect-discord'));

    await waitFor(() => expect(openUrl).toHaveBeenCalledWith('https://discord.com/oauth'));
    // Return route is persisted only after the consent URL launches.
    await waitFor(() => expect(setOAuthReturnRoute).toHaveBeenCalledWith('/rewards'));
    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.auth.oauth_connect',
      params: { provider: 'discord' },
    });
  });

  it('surfaces an error when the RPC returns no oauthUrl', async () => {
    callCoreRpc.mockResolvedValueOnce({ result: {} });
    const { default: RewardsCommunityTab } = await import('../RewardsCommunityTab');
    render(
      <MemoryRouter>
        <RewardsCommunityTab error={null} isLoading={false} snapshot={notLinkedSnapshot()} />
      </MemoryRouter>
    );

    fireEvent.click(screen.getByTestId('rewards-connect-discord'));

    await waitFor(() =>
      expect(screen.getByTestId('rewards-connect-discord-error')).toBeInTheDocument()
    );
    expect(openUrl).not.toHaveBeenCalled();
  });

  it('surfaces an error when the connect RPC rejects', async () => {
    callCoreRpc.mockRejectedValueOnce(new Error('rpc down'));
    const { default: RewardsCommunityTab } = await import('../RewardsCommunityTab');
    render(
      <MemoryRouter>
        <RewardsCommunityTab error={null} isLoading={false} snapshot={notLinkedSnapshot()} />
      </MemoryRouter>
    );

    fireEvent.click(screen.getByTestId('rewards-connect-discord'));

    await waitFor(() =>
      expect(screen.getByTestId('rewards-connect-discord-error')).toBeInTheDocument()
    );
    // A failed initiation must not persist any return route (it's only set after launch).
    expect(setOAuthReturnRoute).not.toHaveBeenCalled();
  });

  it('renders the connected username pill and footer when linked', async () => {
    const { default: RewardsCommunityTab } = await import('../RewardsCommunityTab');
    render(
      <MemoryRouter>
        <RewardsCommunityTab error={null} isLoading={false} snapshot={buildSnapshot()} />
      </MemoryRouter>
    );

    expect(screen.getByTestId('rewards-discord-connected')).toHaveTextContent('cooluser');
    expect(screen.getByTestId('rewards-discord-username')).toHaveTextContent('cooluser');
    expect(screen.queryByTestId('rewards-connect-discord')).not.toBeInTheDocument();
  });
});

describe('RewardsCommunityTab — Disconnect Discord', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('disconnects Discord and refreshes the snapshot', async () => {
    disconnectDiscord.mockResolvedValueOnce(undefined);
    const onRetry = vi.fn();
    const { default: RewardsCommunityTab } = await import('../RewardsCommunityTab');
    render(
      <MemoryRouter>
        <RewardsCommunityTab
          error={null}
          isLoading={false}
          onRetry={onRetry}
          snapshot={buildSnapshot()}
        />
      </MemoryRouter>
    );

    fireEvent.click(screen.getByTestId('rewards-disconnect-discord'));

    await waitFor(() => expect(disconnectDiscord).toHaveBeenCalledTimes(1));
    // Snapshot is refetched so the connected state can flip back to Connect (re-link path).
    await waitFor(() => expect(onRetry).toHaveBeenCalledTimes(1));
    expect(screen.queryByTestId('rewards-disconnect-discord-error')).not.toBeInTheDocument();
  });

  it('surfaces an error and does not refetch when disconnect fails', async () => {
    disconnectDiscord.mockRejectedValueOnce(new Error('disconnect failed'));
    const onRetry = vi.fn();
    const { default: RewardsCommunityTab } = await import('../RewardsCommunityTab');
    render(
      <MemoryRouter>
        <RewardsCommunityTab
          error={null}
          isLoading={false}
          onRetry={onRetry}
          snapshot={buildSnapshot()}
        />
      </MemoryRouter>
    );

    fireEvent.click(screen.getByTestId('rewards-disconnect-discord'));

    await waitFor(() =>
      expect(screen.getByTestId('rewards-disconnect-discord-error')).toBeInTheDocument()
    );
    expect(onRetry).not.toHaveBeenCalled();
  });
});

describe('RewardsCommunityTab — Discord role assignment', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('shows an assigned badge and the assigned-count for an in-guild member', async () => {
    // buildSnapshot: member, role-1 unlocked + assigned, role-2 locked.
    const { default: RewardsCommunityTab } = await import('../RewardsCommunityTab');
    render(
      <MemoryRouter>
        <RewardsCommunityTab error={null} isLoading={false} snapshot={buildSnapshot()} />
      </MemoryRouter>
    );

    expect(screen.getByTestId('rewards-role-status-role-1')).toHaveTextContent('Role assigned');
    // Locked achievements have no role to claim yet, so no badge.
    expect(screen.queryByTestId('rewards-role-status-role-2')).not.toBeInTheDocument();
    expect(screen.getByTestId('rewards-roles-assigned')).toHaveTextContent('1 of 1 roles assigned');
    // Already in the guild -> no join-to-claim prompt.
    expect(screen.queryByTestId('rewards-claim-roles-banner')).not.toBeInTheDocument();
  });

  it('shows a pending badge when an unlocked achievement has no role assigned yet', async () => {
    const snapshot = buildSnapshot();
    snapshot.achievements[0].discordRoleStatus = 'not_assigned';
    const { default: RewardsCommunityTab } = await import('../RewardsCommunityTab');
    render(
      <MemoryRouter>
        <RewardsCommunityTab error={null} isLoading={false} snapshot={snapshot} />
      </MemoryRouter>
    );

    expect(screen.getByTestId('rewards-role-status-role-1')).toHaveTextContent('Syncing role');
  });

  it('prompts a connected non-member to join the server to claim unlocked roles', async () => {
    const snapshot = buildSnapshot();
    snapshot.discord.membershipStatus = 'not_in_guild';
    snapshot.achievements[0].discordRoleStatus = 'not_in_guild';
    const { default: RewardsCommunityTab } = await import('../RewardsCommunityTab');
    render(
      <MemoryRouter>
        <RewardsCommunityTab error={null} isLoading={false} snapshot={snapshot} />
      </MemoryRouter>
    );

    expect(screen.getByTestId('rewards-claim-roles-banner')).toBeInTheDocument();
    expect(screen.getByTestId('rewards-role-status-role-1')).toHaveTextContent(
      'Join server to claim'
    );
    // The member-only assigned-count row is hidden when the user is not in the guild.
    expect(screen.queryByTestId('rewards-roles-assigned')).not.toBeInTheDocument();

    fireEvent.click(screen.getByTestId('rewards-claim-roles-join'));
    expect(openUrl).toHaveBeenCalledWith('https://discord.gg/example');
  });

  it('hides role-assignment status entirely when Discord is not linked', async () => {
    const snapshot = buildSnapshot();
    snapshot.discord = {
      linked: false,
      discordId: null,
      username: null,
      inviteUrl: 'https://discord.gg/example',
      membershipStatus: 'not_linked',
    };
    const { default: RewardsCommunityTab } = await import('../RewardsCommunityTab');
    render(
      <MemoryRouter>
        <RewardsCommunityTab error={null} isLoading={false} snapshot={snapshot} />
      </MemoryRouter>
    );

    expect(screen.queryByTestId('rewards-role-status-role-1')).not.toBeInTheDocument();
    expect(screen.queryByTestId('rewards-claim-roles-banner')).not.toBeInTheDocument();
    expect(screen.queryByTestId('rewards-roles-assigned')).not.toBeInTheDocument();
  });
});
