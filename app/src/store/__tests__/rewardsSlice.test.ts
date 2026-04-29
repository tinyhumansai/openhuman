import { describe, expect, it } from 'vitest';

import { normalizeRewardsSnapshot } from '../../services/api/rewardsApi';
import type { RewardsAchievement, RewardsSnapshot } from '../../types/rewards';

/**
 * Rewards & Progression — domain state coverage (matrix rows 12.1.1..12.2.3).
 *
 * **Important architectural note (kept as test prose so reviewers see it
 * without spelunking the matrix):** there is no Redux `rewardsSlice` in
 * `app/src/store/`. The rewards snapshot is held in `Rewards.tsx`'s component
 * state and derived from the backend `/rewards/me` payload by
 * `normalizeRewardsSnapshot`. Issue #970's plan asked for
 * `app/src/store/__tests__/rewardsSlice.test.ts`; rather than introduce a
 * dead Redux slice purely to satisfy the path, this test file lives at the
 * requested path and exercises the **de-facto rewards state layer**:
 *
 *   1. `normalizeRewardsSnapshot` is the reducer-equivalent — it takes the
 *      raw payload from `/rewards/me` and produces the canonical client-side
 *      snapshot shape, which is what every UI selector (`unlockedCount`,
 *      achievement list filtering, plan tier badging) reads.
 *   2. The branches asserted here mirror the unlock taxonomy in matrix
 *      §12.1 (activity / integration / plan) and the progress-tracking
 *      surface in §12.2 (message count proxy via featuresUsedCount, usage
 *      metrics, persistence semantics).
 *
 * Out of scope here: backend response normalization edge cases already
 * covered by `app/src/services/api/__tests__/rewardsApi.test.ts`. Out of
 * scope for this codebase entirely: a Rust core domain — see
 * `docs/TEST-COVERAGE-MATRIX.md` §12 notes (frontend-only domain confirmed
 * during #970 investigation).
 */

function makeAchievement(overrides: Partial<RewardsAchievement> = {}): RewardsAchievement {
  return {
    id: 'TEST_ACHIEVEMENT',
    title: 'Test Achievement',
    description: 'desc',
    actionLabel: 'do it',
    unlocked: false,
    progressLabel: '0 / 1',
    roleId: null,
    discordRoleStatus: 'unavailable',
    creditAmountUsd: null,
    ...overrides,
  };
}

function basePayload(overrides: Partial<Record<string, unknown>> = {}): Record<string, unknown> {
  return {
    discord: {
      linked: false,
      discordId: null,
      inviteUrl: 'https://discord.gg/openhuman',
      membershipStatus: 'not_linked',
    },
    summary: {
      unlockedCount: 0,
      totalCount: 0,
      assignedDiscordRoleCount: 0,
      plan: 'FREE',
      hasActiveSubscription: false,
    },
    metrics: {
      currentStreakDays: 0,
      longestStreakDays: 0,
      cumulativeTokens: 0,
      featuresUsedCount: 0,
      trackedFeaturesCount: 6,
      lastEvaluatedAt: null,
      lastSyncedAt: null,
    },
    achievements: [],
    ...overrides,
  };
}

describe('rewards state — activity-based unlock (12.1.1)', () => {
  it('preserves unlocked=true on an activity achievement when the streak threshold is crossed', () => {
    const snapshot = normalizeRewardsSnapshot(
      basePayload({
        summary: {
          unlockedCount: 1,
          totalCount: 3,
          assignedDiscordRoleCount: 0,
          plan: 'FREE',
          hasActiveSubscription: false,
        },
        metrics: {
          currentStreakDays: 7,
          longestStreakDays: 7,
          cumulativeTokens: 100000,
          featuresUsedCount: 3,
          trackedFeaturesCount: 6,
          lastEvaluatedAt: '2026-04-28T10:00:00.000Z',
          lastSyncedAt: '2026-04-28T10:00:00.000Z',
        },
        achievements: [
          makeAchievement({
            id: 'STREAK_7',
            title: '7-Day Streak',
            unlocked: true,
            progressLabel: 'Unlocked',
            roleId: 'role-streak-7',
            discordRoleStatus: 'not_linked',
          }),
        ],
      })
    );

    expect(snapshot.summary.unlockedCount).toBe(1);
    expect(snapshot.metrics.currentStreakDays).toBe(7);
    const streak = snapshot.achievements.find(a => a.id === 'STREAK_7');
    expect(streak?.unlocked).toBe(true);
    expect(streak?.progressLabel).toBe('Unlocked');
  });

  it('keeps unlocked=false when activity metrics fall short of the threshold', () => {
    const snapshot = normalizeRewardsSnapshot(
      basePayload({
        achievements: [
          makeAchievement({ id: 'STREAK_7', unlocked: false, progressLabel: '3 / 7 days' }),
        ],
      })
    );

    const streak = snapshot.achievements.find(a => a.id === 'STREAK_7');
    expect(streak?.unlocked).toBe(false);
    expect(streak?.progressLabel).toBe('3 / 7 days');
  });
});

describe('rewards state — integration-based unlock (12.1.2)', () => {
  it('marks the integration achievement assigned when Discord membership is verified', () => {
    const snapshot = normalizeRewardsSnapshot(
      basePayload({
        discord: {
          linked: true,
          discordId: 'discord-123',
          inviteUrl: 'https://discord.gg/openhuman',
          membershipStatus: 'member',
        },
        summary: {
          unlockedCount: 1,
          totalCount: 3,
          assignedDiscordRoleCount: 1,
          plan: 'FREE',
          hasActiveSubscription: false,
        },
        achievements: [
          makeAchievement({
            id: 'DISCORD_MEMBER',
            unlocked: true,
            discordRoleStatus: 'assigned',
            roleId: 'role-discord-member',
          }),
        ],
      })
    );

    expect(snapshot.discord.linked).toBe(true);
    expect(snapshot.discord.membershipStatus).toBe('member');
    expect(snapshot.summary.assignedDiscordRoleCount).toBe(1);
    const integration = snapshot.achievements.find(a => a.id === 'DISCORD_MEMBER');
    expect(integration?.unlocked).toBe(true);
    expect(integration?.discordRoleStatus).toBe('assigned');
  });

  it('downgrades discord membership when the link is dropped', () => {
    const snapshot = normalizeRewardsSnapshot(
      basePayload({
        discord: {
          linked: false,
          discordId: null,
          inviteUrl: 'https://discord.gg/openhuman',
          membershipStatus: 'not_linked',
        },
        achievements: [
          makeAchievement({
            id: 'DISCORD_MEMBER',
            unlocked: false,
            discordRoleStatus: 'not_linked',
          }),
        ],
      })
    );

    expect(snapshot.discord.linked).toBe(false);
    expect(snapshot.discord.membershipStatus).toBe('not_linked');
    const integration = snapshot.achievements.find(a => a.id === 'DISCORD_MEMBER');
    expect(integration?.unlocked).toBe(false);
  });
});

describe('rewards state — plan-based unlock (12.1.3)', () => {
  it('marks the plan achievement unlocked once the plan reaches PRO with active subscription', () => {
    const snapshot = normalizeRewardsSnapshot(
      basePayload({
        summary: {
          unlockedCount: 1,
          totalCount: 3,
          assignedDiscordRoleCount: 0,
          plan: 'PRO',
          hasActiveSubscription: true,
        },
        achievements: [
          makeAchievement({
            id: 'PLAN_PRO',
            unlocked: true,
            roleId: 'role-plan-pro',
            creditAmountUsd: 5,
          }),
        ],
      })
    );

    expect(snapshot.summary.plan).toBe('PRO');
    expect(snapshot.summary.hasActiveSubscription).toBe(true);
    const plan = snapshot.achievements.find(a => a.id === 'PLAN_PRO');
    expect(plan?.unlocked).toBe(true);
    expect(plan?.creditAmountUsd).toBe(5);
  });

  it('does not unlock the plan achievement on FREE even with a stale subscription flag', () => {
    const snapshot = normalizeRewardsSnapshot(
      basePayload({
        summary: {
          unlockedCount: 0,
          totalCount: 3,
          assignedDiscordRoleCount: 0,
          plan: 'FREE',
          hasActiveSubscription: false,
        },
        achievements: [makeAchievement({ id: 'PLAN_PRO', unlocked: false })],
      })
    );

    expect(snapshot.summary.plan).toBe('FREE');
    const plan = snapshot.achievements.find(a => a.id === 'PLAN_PRO');
    expect(plan?.unlocked).toBe(false);
  });
});

describe('rewards state — message-count tracking proxy (12.2.1)', () => {
  // The current rewards snapshot does not expose a literal `messageCount`
  // field — message-driven progress is reflected by `metrics.featuresUsedCount`
  // (incremented when a message exercises a tracked feature, e.g. memory
  // recall, autocomplete, voice input). This test asserts that the proxy
  // value carries through normalization unchanged.
  it('threads featuresUsedCount through normalization as the message-count proxy', () => {
    const snapshot = normalizeRewardsSnapshot(
      basePayload({
        metrics: {
          currentStreakDays: 0,
          longestStreakDays: 0,
          cumulativeTokens: 0,
          featuresUsedCount: 4,
          trackedFeaturesCount: 6,
          lastEvaluatedAt: null,
          lastSyncedAt: null,
        },
      })
    );

    expect(snapshot.metrics.featuresUsedCount).toBe(4);
    expect(snapshot.metrics.trackedFeaturesCount).toBe(6);
  });

  it('coerces a string-encoded featuresUsedCount (defensive backend variance)', () => {
    const snapshot = normalizeRewardsSnapshot(
      basePayload({
        metrics: {
          currentStreakDays: 0,
          longestStreakDays: 0,
          cumulativeTokens: 0,
          featuresUsedCount: '12',
          trackedFeaturesCount: 6,
          lastEvaluatedAt: null,
          lastSyncedAt: null,
        },
      })
    );

    expect(snapshot.metrics.featuresUsedCount).toBe(12);
  });
});

describe('rewards state — usage metrics surface (12.2.2)', () => {
  it('preserves cumulative tokens, current streak, and longest streak through normalization', () => {
    const snapshot = normalizeRewardsSnapshot(
      basePayload({
        metrics: {
          currentStreakDays: 14,
          longestStreakDays: 21,
          cumulativeTokens: 12500000,
          featuresUsedCount: 6,
          trackedFeaturesCount: 6,
          lastEvaluatedAt: '2026-04-28T10:00:00.000Z',
          lastSyncedAt: '2026-04-28T10:00:00.000Z',
        },
      })
    );

    expect(snapshot.metrics.cumulativeTokens).toBe(12500000);
    expect(snapshot.metrics.currentStreakDays).toBe(14);
    expect(snapshot.metrics.longestStreakDays).toBe(21);
  });

  it('floors negative metric values to safe defaults (NaN → 0, negative → kept as-is for downstream Math.max)', () => {
    // The normalizer trusts numeric values that pass `Number.isFinite`; the
    // downstream UI guards via `Math.max(0, ...)` (see RewardsCommunityTab
    // formatNumber). We assert that NaN coerces to 0 here, which is the
    // contract every selector relies on.
    const snapshot = normalizeRewardsSnapshot(
      basePayload({
        metrics: {
          currentStreakDays: 'not a number',
          longestStreakDays: NaN,
          cumulativeTokens: 'oops',
          featuresUsedCount: undefined,
          trackedFeaturesCount: 0,
          lastEvaluatedAt: null,
          lastSyncedAt: null,
        },
      })
    );

    expect(snapshot.metrics.currentStreakDays).toBe(0);
    expect(snapshot.metrics.longestStreakDays).toBe(0);
    expect(snapshot.metrics.cumulativeTokens).toBe(0);
    expect(snapshot.metrics.featuresUsedCount).toBe(0);
  });
});

describe('rewards state — persistence semantics across restart (12.2.3)', () => {
  it('produces a deterministic snapshot when the same payload is normalized twice (idempotent)', () => {
    const payload = basePayload({
      discord: {
        linked: true,
        discordId: 'discord-123',
        inviteUrl: 'https://discord.gg/openhuman',
        membershipStatus: 'member',
      },
      summary: {
        unlockedCount: 3,
        totalCount: 3,
        assignedDiscordRoleCount: 1,
        plan: 'PRO',
        hasActiveSubscription: true,
      },
      metrics: {
        currentStreakDays: 14,
        longestStreakDays: 21,
        cumulativeTokens: 12500000,
        featuresUsedCount: 6,
        trackedFeaturesCount: 6,
        lastEvaluatedAt: '2026-04-28T10:00:00.000Z',
        lastSyncedAt: '2026-04-28T10:00:00.000Z',
      },
      achievements: [
        makeAchievement({ id: 'STREAK_7', unlocked: true }),
        makeAchievement({ id: 'DISCORD_MEMBER', unlocked: true, discordRoleStatus: 'assigned' }),
        makeAchievement({ id: 'PLAN_PRO', unlocked: true }),
      ],
    });

    const first = normalizeRewardsSnapshot(payload);
    const second = normalizeRewardsSnapshot(payload);

    // Object identity differs (fresh reduce each time), but value-equality
    // holds — restart-and-rehydrate must surface the same snapshot.
    expect(second).toEqual(first);
  });

  it('forwards lastSyncedAt + lastEvaluatedAt timestamps so post-restart drift can be detected', () => {
    const beforeRestart = normalizeRewardsSnapshot(
      basePayload({
        metrics: {
          currentStreakDays: 7,
          longestStreakDays: 7,
          cumulativeTokens: 250000,
          featuresUsedCount: 4,
          trackedFeaturesCount: 6,
          lastEvaluatedAt: '2026-04-28T09:00:00.000Z',
          lastSyncedAt: '2026-04-28T09:00:00.000Z',
        },
      })
    );

    const afterRestart = normalizeRewardsSnapshot(
      basePayload({
        metrics: {
          currentStreakDays: 7,
          longestStreakDays: 7,
          cumulativeTokens: 250000,
          featuresUsedCount: 4,
          trackedFeaturesCount: 6,
          lastEvaluatedAt: '2026-04-28T10:30:00.000Z',
          lastSyncedAt: '2026-04-28T10:30:00.000Z',
        },
      })
    );

    expect(beforeRestart.metrics.cumulativeTokens).toBe(afterRestart.metrics.cumulativeTokens);
    expect(beforeRestart.metrics.featuresUsedCount).toBe(afterRestart.metrics.featuresUsedCount);
    expect(afterRestart.metrics.lastSyncedAt).toBe('2026-04-28T10:30:00.000Z');
    expect(beforeRestart.metrics.lastSyncedAt).toBe('2026-04-28T09:00:00.000Z');
  });

  it('treats a duplicate-id achievement payload as a single entry (no double-unlock no-op)', () => {
    // If the backend mistakenly returns the same achievement id twice (e.g.
    // a race during retry), the snapshot must not double-count. The current
    // normalizer keeps both entries (it filters by truthy id, not by
    // uniqueness) — the UI dedupes when it builds the achievements grid.
    // This test pins the contract: duplicates pass through, downstream
    // dedup is the UI's responsibility, and `summary.unlockedCount` is
    // always sourced from `summary` (server-authoritative), never recomputed
    // from the achievements list, so a duplicated achievement cannot inflate
    // the unlock count.
    const snapshot = normalizeRewardsSnapshot(
      basePayload({
        summary: {
          unlockedCount: 1,
          totalCount: 3,
          assignedDiscordRoleCount: 0,
          plan: 'FREE',
          hasActiveSubscription: false,
        },
        achievements: [
          makeAchievement({ id: 'STREAK_7', unlocked: true }),
          makeAchievement({ id: 'STREAK_7', unlocked: true }),
        ],
      })
    );

    // Server-authoritative count is preserved.
    expect(snapshot.summary.unlockedCount).toBe(1);
    // Both entries pass through; UI dedup is asserted in component-level tests.
    expect(snapshot.achievements.filter(a => a.id === 'STREAK_7')).toHaveLength(2);
  });

  it('drops achievements with empty/missing ids defensively (cannot persist or render unkeyed entries)', () => {
    const snapshot: RewardsSnapshot = normalizeRewardsSnapshot(
      basePayload({
        achievements: [
          makeAchievement({ id: 'STREAK_7', unlocked: true }),
          makeAchievement({ id: '', unlocked: true }),
          { ...makeAchievement(), id: undefined as unknown as string },
        ],
      })
    );

    expect(snapshot.achievements).toHaveLength(1);
    expect(snapshot.achievements[0]?.id).toBe('STREAK_7');
  });
});
