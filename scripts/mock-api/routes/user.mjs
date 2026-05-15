import { json } from "../http.mjs";
import { behavior, getMockTeam } from "../state.mjs";

export function handleUser(ctx) {
  const { method, url, res, origin } = ctx;
  const mockBehavior = behavior();

  if (method === "GET" && /^\/settings\/?(\?.*)?$/.test(url)) {
    json(res, 200, {
      success: true,
      data: { _id: "e2e-user-1", username: "e2e" },
    });
    return true;
  }

  if (method === "GET" && /^\/teams\/?(\?.*)?$/.test(url)) {
    json(res, 200, { success: true, data: [getMockTeam()] });
    return true;
  }

  if (method === "GET" && /^\/teams\/me\/usage\/?(\?.*)?$/.test(url)) {
    json(res, 200, {
      success: true,
      data: {
        cycleBudgetUsd: 10,
        remainingUsd: 10,
        cycleLimit5hr: 0,
        cycleLimit7day: 0,
        fiveHourCapUsd: 5,
        fiveHourResetsAt: null,
        cycleStartDate: new Date().toISOString(),
        cycleEndsAt: new Date(
          Date.now() + 7 * 24 * 60 * 60 * 1000,
        ).toISOString(),
        bypassCycleLimit: false,
      },
    });
    return true;
  }

  if (method === "POST" && /^\/teams\/join\/?$/.test(url)) {
    // Gap fill: accept team invite.
    json(res, 200, { success: true, data: getMockTeam() });
    return true;
  }

  if (method === "GET" && /^\/users\/?(\?.*)?$/.test(url)) {
    // Gap fill: list users (admin/team context). Empty list keeps the UI quiet.
    json(res, 200, { success: true, data: [] });
    return true;
  }

  if (
    method === "POST" &&
    /^\/telegram\/settings\/onboarding-complete\/?$/.test(url)
  ) {
    json(res, 200, { success: true, data: {} });
    return true;
  }
  if (method === "POST" && /^\/settings\/onboarding-complete\/?$/.test(url)) {
    json(res, 200, { success: true, data: {} });
    return true;
  }

  if (method === "GET" && /^\/referral\/stats\/?(\?.*)?$/.test(url)) {
    json(res, 200, {
      success: true,
      data: {
        referralCode: "MOCKREF1",
        referralLink: `${origin}/#/rewards?ref=MOCKREF1`,
        totals: {
          totalRewardUsd: 10,
          pendingCount: 1,
          convertedCount: 2,
        },
        referrals: [
          {
            id: "ref-row-1",
            referredUserId: "user-456",
            status: "pending",
            createdAt: new Date(Date.now() - 86400000).toISOString(),
          },
          {
            id: "ref-row-2",
            referredUserId: "user-789",
            status: "converted",
            createdAt: new Date(Date.now() - 172800000).toISOString(),
            convertedAt: new Date().toISOString(),
            rewardUsd: 5,
          },
        ],
        appliedReferralCode: null,
        canApplyReferral: true,
      },
    });
    return true;
  }

  if (method === "POST" && /^\/referral\/claim\/?$/.test(url)) {
    json(res, 200, {
      success: true,
      data: { ok: true, message: "Referral claimed" },
    });
    return true;
  }

  if (method === "GET" && /^\/rewards\/me\/?(\?.*)?$/.test(url)) {
    if (mockBehavior.rewardsServiceError === "true") {
      json(res, 503, {
        success: false,
        error: "Rewards service unavailable",
      });
      return true;
    }
    json(res, 200, { success: true, data: buildRewardsSnapshot(mockBehavior) });
    return true;
  }

  return false;
}

function buildRewardsSnapshot(mockBehavior) {
  const scenario = mockBehavior.rewardsScenario || "default";
  const lastSyncedAt =
    mockBehavior.rewardsLastSyncedAt || new Date().toISOString();

  const baseAchievements = [
    {
      id: "STREAK_7",
      title: "7-Day Streak",
      description: "Use OpenHuman on seven consecutive active days.",
      actionLabel: "Keep your streak alive for 7 days",
      unlocked: false,
      progressLabel: "0 / 7 days",
      roleId: "role-streak-7",
      discordRoleStatus: "not_linked",
      creditAmountUsd: null,
    },
    {
      id: "DISCORD_MEMBER",
      title: "Discord Member",
      description: "Join the OpenHuman Discord server.",
      actionLabel: "Connect Discord and join the server",
      unlocked: false,
      progressLabel: "Not joined",
      roleId: "role-discord-member",
      discordRoleStatus: "not_linked",
      creditAmountUsd: null,
    },
    {
      id: "PLAN_PRO",
      title: "Pro Supporter",
      description: "Upgrade to the Pro plan.",
      actionLabel: "Upgrade to Pro",
      unlocked: false,
      progressLabel: "Locked",
      roleId: "role-plan-pro",
      discordRoleStatus: "not_assigned",
      creditAmountUsd: 5,
    },
  ];

  const defaultDiscord = {
    linked: false,
    discordId: null,
    inviteUrl: "https://discord.gg/openhuman",
    membershipStatus: "not_linked",
  };
  const memberDiscord = {
    linked: true,
    discordId: "discord-mock-123",
    inviteUrl: "https://discord.gg/openhuman",
    membershipStatus: "member",
  };
  const zeroMetrics = {
    currentStreakDays: 0,
    longestStreakDays: 0,
    cumulativeTokens: 0,
    featuresUsedCount: 0,
    trackedFeaturesCount: 6,
    lastEvaluatedAt: lastSyncedAt,
    lastSyncedAt,
  };

  switch (scenario) {
    case "activity_unlocked":
      return {
        discord: defaultDiscord,
        summary: {
          unlockedCount: 1,
          totalCount: 3,
          assignedDiscordRoleCount: 0,
          plan: "FREE",
          hasActiveSubscription: false,
        },
        metrics: {
          ...zeroMetrics,
          currentStreakDays: 7,
          longestStreakDays: 7,
          cumulativeTokens: 250000,
          featuresUsedCount: 4,
        },
        achievements: [
          {
            ...baseAchievements[0],
            unlocked: true,
            progressLabel: "Unlocked",
            discordRoleStatus: "not_linked",
          },
          baseAchievements[1],
          baseAchievements[2],
        ],
      };
    case "integration_unlocked":
      return {
        discord: memberDiscord,
        summary: {
          unlockedCount: 1,
          totalCount: 3,
          assignedDiscordRoleCount: 1,
          plan: "FREE",
          hasActiveSubscription: false,
        },
        metrics: { ...zeroMetrics },
        achievements: [
          baseAchievements[0],
          {
            ...baseAchievements[1],
            unlocked: true,
            progressLabel: "Unlocked",
            discordRoleStatus: "assigned",
          },
          baseAchievements[2],
        ],
      };
    case "plan_unlocked":
      return {
        discord: defaultDiscord,
        summary: {
          unlockedCount: 1,
          totalCount: 3,
          assignedDiscordRoleCount: 0,
          plan: "PRO",
          hasActiveSubscription: true,
        },
        metrics: { ...zeroMetrics },
        achievements: [
          baseAchievements[0],
          baseAchievements[1],
          {
            ...baseAchievements[2],
            unlocked: true,
            progressLabel: "Unlocked",
            discordRoleStatus: "not_linked",
          },
        ],
      };
    case "high_usage":
    case "post_restart":
      return {
        discord: memberDiscord,
        summary: {
          unlockedCount: 3,
          totalCount: 3,
          assignedDiscordRoleCount: 1,
          plan: "PRO",
          hasActiveSubscription: true,
        },
        metrics: {
          ...zeroMetrics,
          currentStreakDays: 14,
          longestStreakDays: 21,
          cumulativeTokens: 12500000,
          featuresUsedCount: 6,
        },
        achievements: baseAchievements.map((a) => ({
          ...a,
          unlocked: true,
          progressLabel: "Unlocked",
          discordRoleStatus: "assigned",
        })),
      };
    case "default":
    default:
      return {
        discord: defaultDiscord,
        summary: {
          unlockedCount: 0,
          totalCount: 3,
          assignedDiscordRoleCount: 0,
          plan: "FREE",
          hasActiveSubscription: false,
        },
        metrics: { ...zeroMetrics },
        achievements: baseAchievements,
      };
  }
}
