/**
 * Smoke test for `RewardsCommunityTab` — exercises the `role.unlocked`
 * branch (line 248) added by PR #2095's dark-mode pass so the diff
 * coverage gate has the touched line covered.
 */
import { render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';

import type { RewardsSnapshot } from '../../../types/rewards';

vi.mock('../../../utils/openUrl', () => ({ openUrl: vi.fn() }));

function buildSnapshot(): RewardsSnapshot {
  return {
    discord: {
      linked: true,
      discordId: 'discord-1',
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
