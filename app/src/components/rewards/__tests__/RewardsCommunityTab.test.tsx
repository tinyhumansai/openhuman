/**
 * Smoke test for `RewardsCommunityTab` — exercises the `role.unlocked`
 * branch (line 248) added by PR #2095's dark-mode pass so the diff
 * coverage gate has the touched line covered.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { RewardsSnapshot } from '../../../types/rewards';

const { openUrl, callCoreRpc, setOAuthReturnRoute, disconnectDiscord, claimReward } = vi.hoisted(
  () => ({
    openUrl: vi.fn(),
    callCoreRpc: vi.fn(),
    setOAuthReturnRoute: vi.fn(),
    disconnectDiscord: vi.fn(),
    claimReward: vi.fn(),
  })
);

vi.mock('../../../utils/openUrl', () => ({ openUrl }));
vi.mock('../../../services/coreRpcClient', () => ({ callCoreRpc }));
vi.mock('../../../services/api/rewardsApi', () => ({
  rewardsApi: { disconnectDiscord, claimReward },
}));
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
        rewardTokens: 500000,
        rewardRecurring: false,
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
        rewardTokens: 2000000,
        rewardRecurring: false,
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
    await waitFor(() => expect(setOAuthReturnRoute).toHaveBeenCalledWith('/rewards?view=main'));
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

describe('RewardsCommunityTab — Claim reward', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  function claimableSnapshot(): RewardsSnapshot {
    const snapshot = buildSnapshot();
    // role-1: unlocked + claimable (not yet claimed). role-2 stays locked.
    snapshot.achievements[0] = { ...snapshot.achievements[0], claimable: true, claimed: false };
    return snapshot;
  }

  function claimedSnapshot(): RewardsSnapshot {
    const snapshot = buildSnapshot();
    // Server truth after a claim: no longer claimable, now claimed.
    snapshot.achievements[0] = { ...snapshot.achievements[0], claimable: false, claimed: true };
    return snapshot;
  }

  const claimResult = (over: Record<string, unknown> = {}) => ({
    reward: 'role-1',
    recurring: false,
    period: null,
    tokens: 500000,
    amountUsd: 1.25,
    alreadyClaimed: false,
    claimedAt: '2026-07-03T00:00:00.000Z',
    newPromoBalanceUsd: 5.25,
    ...over,
  });

  it('shows a Claim button for a claimable reward and hides it for locked ones', async () => {
    const { default: RewardsCommunityTab } = await import('../RewardsCommunityTab');
    render(
      <MemoryRouter>
        <RewardsCommunityTab error={null} isLoading={false} snapshot={claimableSnapshot()} />
      </MemoryRouter>
    );

    expect(screen.getByTestId('rewards-claim-role-1')).toHaveTextContent('Claim 500K tokens');
    // Locked role-2 is neither claimable nor claimed -> no claim footer.
    expect(screen.queryByTestId('rewards-claim-role-2')).not.toBeInTheDocument();
    expect(screen.queryByTestId('rewards-claimed-role-2')).not.toBeInTheDocument();
  });

  it('claims a reward, triggers a silent refresh, and shows the credited amount once the server confirms', async () => {
    claimReward.mockResolvedValueOnce(claimResult());
    const onSilentRefresh = vi.fn().mockResolvedValue(undefined);
    const { default: RewardsCommunityTab } = await import('../RewardsCommunityTab');
    const { rerender } = render(
      <MemoryRouter>
        <RewardsCommunityTab
          error={null}
          isLoading={false}
          onSilentRefresh={onSilentRefresh}
          snapshot={claimableSnapshot()}
        />
      </MemoryRouter>
    );

    fireEvent.click(screen.getByTestId('rewards-claim-role-1'));

    await waitFor(() => expect(claimReward).toHaveBeenCalledWith('role-1'));
    // The claim reconciles server truth via a silent refresh (no full-page reload).
    await waitFor(() => expect(onSilentRefresh).toHaveBeenCalledTimes(1));

    // Simulate the refetched snapshot arriving (server marks it claimed).
    rerender(
      <MemoryRouter>
        <RewardsCommunityTab
          error={null}
          isLoading={false}
          onSilentRefresh={onSilentRefresh}
          snapshot={claimedSnapshot()}
        />
      </MemoryRouter>
    );

    expect(screen.getByTestId('rewards-claimed-role-1')).toBeInTheDocument();
    expect(screen.getByTestId('rewards-claim-credited-role-1')).toHaveTextContent(
      '$1.25 credited to your balance'
    );
    expect(screen.queryByTestId('rewards-claim-role-1')).not.toBeInTheDocument();
  });

  it('does not show a fresh-credit note on an idempotent re-claim', async () => {
    claimReward.mockResolvedValueOnce(claimResult({ alreadyClaimed: true }));
    const onSilentRefresh = vi.fn().mockResolvedValue(undefined);
    const { default: RewardsCommunityTab } = await import('../RewardsCommunityTab');
    const { rerender } = render(
      <MemoryRouter>
        <RewardsCommunityTab
          error={null}
          isLoading={false}
          onSilentRefresh={onSilentRefresh}
          snapshot={claimableSnapshot()}
        />
      </MemoryRouter>
    );

    fireEvent.click(screen.getByTestId('rewards-claim-role-1'));
    await waitFor(() => expect(onSilentRefresh).toHaveBeenCalledTimes(1));

    rerender(
      <MemoryRouter>
        <RewardsCommunityTab
          error={null}
          isLoading={false}
          onSilentRefresh={onSilentRefresh}
          snapshot={claimedSnapshot()}
        />
      </MemoryRouter>
    );

    // Claimed pill shows, but no "credited" note — nothing new was credited.
    expect(screen.getByTestId('rewards-claimed-role-1')).toBeInTheDocument();
    expect(screen.queryByTestId('rewards-claim-credited-role-1')).not.toBeInTheDocument();
  });

  it('disables the button and shows a claiming label while the claim is in flight', async () => {
    let resolveClaim: (value: unknown) => void = () => {};
    claimReward.mockImplementationOnce(
      () =>
        new Promise(resolve => {
          resolveClaim = resolve;
        })
    );
    const { default: RewardsCommunityTab } = await import('../RewardsCommunityTab');
    render(
      <MemoryRouter>
        <RewardsCommunityTab error={null} isLoading={false} snapshot={claimableSnapshot()} />
      </MemoryRouter>
    );

    fireEvent.click(screen.getByTestId('rewards-claim-role-1'));

    // In-flight: the button is disabled and shows the claiming label (guards double-submit).
    await waitFor(() => {
      const button = screen.getByTestId('rewards-claim-role-1');
      expect(button).toBeDisabled();
      expect(button).toHaveTextContent('Claiming');
    });

    resolveClaim(claimResult());
  });

  it('tracks in-flight claims per achievement (one pending claim does not re-enable another)', async () => {
    const snapshot = claimableSnapshot();
    // role-2 is also claimable now.
    snapshot.achievements[1] = {
      ...snapshot.achievements[1],
      claimable: true,
      claimed: false,
      rewardTokens: 2000000,
    };
    let resolveRole1: (value: unknown) => void = () => {};
    claimReward
      .mockImplementationOnce(
        () =>
          new Promise(resolve => {
            resolveRole1 = resolve;
          })
      )
      .mockResolvedValueOnce(claimResult({ reward: 'role-2', tokens: 2000000 }));
    const onSilentRefresh = vi.fn().mockResolvedValue(undefined);
    const { default: RewardsCommunityTab } = await import('../RewardsCommunityTab');
    render(
      <MemoryRouter>
        <RewardsCommunityTab
          error={null}
          isLoading={false}
          onSilentRefresh={onSilentRefresh}
          snapshot={snapshot}
        />
      </MemoryRouter>
    );

    // role-1 claim stays pending.
    fireEvent.click(screen.getByTestId('rewards-claim-role-1'));
    await waitFor(() => expect(screen.getByTestId('rewards-claim-role-1')).toBeDisabled());

    // A second claim on role-2 settles fully.
    fireEvent.click(screen.getByTestId('rewards-claim-role-2'));
    await waitFor(() => expect(claimReward).toHaveBeenCalledWith('role-2'));

    // role-1 must remain disabled while its own request is still in flight — a single
    // shared scalar would have re-enabled it when role-2's claim settled.
    await waitFor(() => expect(screen.getByTestId('rewards-claim-role-1')).toBeDisabled());

    resolveRole1(claimResult());
  });

  it('renders a Claimed pill (no button) for an already-claimed reward', async () => {
    const { default: RewardsCommunityTab } = await import('../RewardsCommunityTab');
    render(
      <MemoryRouter>
        <RewardsCommunityTab error={null} isLoading={false} snapshot={claimedSnapshot()} />
      </MemoryRouter>
    );

    expect(screen.getByTestId('rewards-claimed-role-1')).toHaveTextContent('Claimed');
    expect(screen.queryByTestId('rewards-claim-role-1')).not.toBeInTheDocument();
    // No in-session claim -> no credited note on a server-claimed card.
    expect(screen.queryByTestId('rewards-claim-credited-role-1')).not.toBeInTheDocument();
  });

  it('surfaces the backend error message when a claim fails and keeps the button', async () => {
    claimReward.mockRejectedValueOnce({ success: false, error: 'Reward not unlocked yet' });
    const { default: RewardsCommunityTab } = await import('../RewardsCommunityTab');
    render(
      <MemoryRouter>
        <RewardsCommunityTab error={null} isLoading={false} snapshot={claimableSnapshot()} />
      </MemoryRouter>
    );

    fireEvent.click(screen.getByTestId('rewards-claim-role-1'));

    // The actionable backend message is shown, not a generic string.
    await waitFor(() =>
      expect(screen.getByTestId('rewards-claim-error-role-1')).toHaveTextContent(
        'Reward not unlocked yet'
      )
    );
    // Claim did not succeed -> the button is still there to retry.
    expect(screen.getByTestId('rewards-claim-role-1')).toBeInTheDocument();
    expect(screen.queryByTestId('rewards-claimed-role-1')).not.toBeInTheDocument();
  });
});

describe('RewardsCommunityTab — progress badges, progress labels, and stat split', () => {
  it('renders one progress badge per achievement (no 8-item cap)', async () => {
    const snapshot = buildSnapshot();
    // 10 achievements > the old hard cap of 8: every one must get a badge.
    snapshot.achievements = Array.from({ length: 10 }, (_, i) => ({
      id: `ach-${i}`,
      title: `Achievement ${i}`,
      description: `Desc ${i}`,
      actionLabel: 'View',
      unlocked: i < 3,
      progressLabel: i < 3 ? 'Unlocked' : `${i} / 10 active days`,
      roleId: null,
      discordRoleStatus: 'not_configured',
      creditAmountUsd: null,
      rewardTokens: null,
      rewardRecurring: false,
    }));
    const { default: RewardsCommunityTab } = await import('../RewardsCommunityTab');
    render(
      <MemoryRouter>
        <RewardsCommunityTab error={null} isLoading={false} snapshot={snapshot} />
      </MemoryRouter>
    );

    for (let i = 0; i < 10; i++) {
      expect(screen.getByTestId(`rewards-achievement-badge-ach-${i}`)).toBeInTheDocument();
    }
  });

  it('shows the progress label on locked achievements only', async () => {
    // buildSnapshot: role-1 unlocked (progressLabel "1/1"), role-2 locked ("0/1").
    const { default: RewardsCommunityTab } = await import('../RewardsCommunityTab');
    render(
      <MemoryRouter>
        <RewardsCommunityTab error={null} isLoading={false} snapshot={buildSnapshot()} />
      </MemoryRouter>
    );

    expect(screen.getByTestId('rewards-achievement-progress-role-2')).toHaveTextContent('0/1');
    // Unlocked achievements don't show a progress hint.
    expect(screen.queryByTestId('rewards-achievement-progress-role-1')).not.toBeInTheDocument();
  });

  it('separates Discord status from product-activity metrics into two cards', async () => {
    const { default: RewardsCommunityTab } = await import('../RewardsCommunityTab');
    render(
      <MemoryRouter>
        <RewardsCommunityTab error={null} isLoading={false} snapshot={buildSnapshot()} />
      </MemoryRouter>
    );

    const discordCard = screen.getByTestId('rewards-discord-stats');
    const activityCard = screen.getByTestId('rewards-activity-stats');
    // Discord identity lives in the Discord card…
    expect(discordCard).toContainElement(screen.getByTestId('rewards-discord-username'));
    // …and streaks/tokens live in the activity card, no longer mixed in.
    expect(activityCard).toContainElement(screen.getByTestId('rewards-current-streak'));
    expect(activityCard).toContainElement(screen.getByTestId('rewards-longest-streak'));
  });

  it('labels the streak in days and surfaces the longest streak', async () => {
    // buildSnapshot: currentStreakDays 3, longestStreakDays 5.
    const { default: RewardsCommunityTab } = await import('../RewardsCommunityTab');
    render(
      <MemoryRouter>
        <RewardsCommunityTab error={null} isLoading={false} snapshot={buildSnapshot()} />
      </MemoryRouter>
    );

    expect(screen.getByTestId('rewards-current-streak')).toHaveTextContent('3 days');
    expect(screen.getByTestId('rewards-longest-streak')).toHaveTextContent('5 days');
  });

  it('shows the token reward pill with a compact amount', async () => {
    // buildSnapshot: role-1 rewardTokens 500000, role-2 rewardTokens 2000000.
    const { default: RewardsCommunityTab } = await import('../RewardsCommunityTab');
    render(
      <MemoryRouter>
        <RewardsCommunityTab error={null} isLoading={false} snapshot={buildSnapshot()} />
      </MemoryRouter>
    );

    expect(screen.getByTestId('rewards-achievement-reward-role-1')).toHaveTextContent(
      '+500K tokens'
    );
    expect(screen.getByTestId('rewards-achievement-reward-role-2')).toHaveTextContent('+2M tokens');
  });

  it('shows a per-month reward pill for recurring subscriber rewards', async () => {
    const snapshot = buildSnapshot();
    snapshot.achievements = [
      {
        id: 'sub-1',
        title: 'Soft Launch',
        description: 'Monthly subscriber.',
        actionLabel: 'View',
        unlocked: true,
        progressLabel: 'Unlocked',
        roleId: null,
        discordRoleStatus: 'not_configured',
        creditAmountUsd: null,
        rewardTokens: 5000000,
        rewardRecurring: true,
      },
    ];
    const { default: RewardsCommunityTab } = await import('../RewardsCommunityTab');
    render(
      <MemoryRouter>
        <RewardsCommunityTab error={null} isLoading={false} snapshot={snapshot} />
      </MemoryRouter>
    );

    expect(screen.getByTestId('rewards-achievement-reward-sub-1')).toHaveTextContent(
      '+5M tokens/mo'
    );
  });

  it('counts only assignable (unlocked + role-configured) achievements in the roles ratio', async () => {
    const snapshot = buildSnapshot();
    // Two unlocked achievements, but only one has a configured Discord role. The
    // role-less one can never be assigned, so it must not inflate the denominator.
    snapshot.achievements = [
      {
        id: 'role-a',
        title: 'Has role',
        description: 'Unlocked with a configured Discord role, assigned.',
        actionLabel: 'View',
        unlocked: true,
        progressLabel: 'Unlocked',
        roleId: 'discord-role-a',
        discordRoleStatus: 'assigned',
        creditAmountUsd: null,
        rewardTokens: null,
        rewardRecurring: false,
      },
      {
        id: 'role-b',
        title: 'No role',
        description: 'Unlocked but no Discord role configured.',
        actionLabel: 'View',
        unlocked: true,
        progressLabel: 'Unlocked',
        roleId: null,
        discordRoleStatus: 'not_configured',
        creditAmountUsd: null,
        rewardTokens: null,
        rewardRecurring: false,
      },
    ];
    const { default: RewardsCommunityTab } = await import('../RewardsCommunityTab');
    render(
      <MemoryRouter>
        <RewardsCommunityTab error={null} isLoading={false} snapshot={snapshot} />
      </MemoryRouter>
    );

    // 2 unlocked, 1 assignable, 1 assigned → "1 of 1", never "1 of 2".
    expect(screen.getByTestId('rewards-roles-assigned')).toHaveTextContent('1 of 1 roles assigned');
  });
});
