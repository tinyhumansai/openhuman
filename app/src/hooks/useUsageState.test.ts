import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const mockGetCurrentPlan = vi.fn();
const mockGetTeamUsage = vi.fn();

vi.mock('../services/api/billingApi', () => ({
  billingApi: { getCurrentPlan: () => mockGetCurrentPlan() },
}));

vi.mock('../services/api/creditsApi', () => ({
  creditsApi: { getTeamUsage: () => mockGetTeamUsage() },
}));

interface BuildUsageOpts {
  remainingUsd?: number;
  cycleBudgetUsd?: number;
  cycleSpentUsd?: number;
}

function buildUsage(opts: BuildUsageOpts = {}) {
  const cycleBudgetUsd = opts.cycleBudgetUsd ?? 0;
  const remainingUsd = opts.remainingUsd ?? 0;
  return {
    remainingUsd,
    cycleBudgetUsd,
    cycleSpentUsd: opts.cycleSpentUsd ?? Math.max(0, cycleBudgetUsd - remainingUsd),
    cycleStartDate: '2026-04-09T00:00:00.000Z',
    cycleEndsAt: '2026-04-16T00:00:00.000Z',
    plan: {
      plan: 'FREE',
      name: 'Free',
      marginPercent: 50,
      payAsYouGoMarginPercent: 50,
      discountVsPayAsYouGoPercent: 0,
    },
    insights: {
      period: { startDate: '2026-04-09T00:00:00.000Z', endDate: '2026-04-16T00:00:00.000Z' },
      totals: {
        inferenceUsd: 0,
        integrationsUsd: 0,
        totalUsd: 0,
        inferenceCalls: 0,
        integrationCalls: 0,
      },
      dailySeries: [],
      topModels: [],
      topIntegrations: [],
    },
  };
}

function freePlan() {
  return {
    plan: 'FREE' as const,
    hasActiveSubscription: false,
    planExpiry: null,
    subscription: null,
    monthlyBudgetUsd: 0,
    weeklyBudgetUsd: 0,
  };
}

function basicPlan() {
  return {
    plan: 'BASIC' as const,
    hasActiveSubscription: true,
    planExpiry: '2026-05-01T00:00:00.000Z',
    subscription: {
      id: 'sub_123',
      status: 'active',
      currentPeriodEnd: '2026-05-01T00:00:00.000Z',
      quantity: 1,
    },
    monthlyBudgetUsd: 20,
    weeklyBudgetUsd: 10,
  };
}

describe('useUsageState', () => {
  beforeEach(() => {
    vi.resetModules();
    mockGetCurrentPlan.mockReset();
    mockGetTeamUsage.mockReset();
  });

  it('does not treat free users with zero recurring budget as exhausted', async () => {
    const { useUsageState } = await import('./useUsageState');
    mockGetCurrentPlan.mockResolvedValue(freePlan());
    mockGetTeamUsage.mockResolvedValue(buildUsage());

    const { result } = renderHook(() => useUsageState());

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
    });

    expect(result.current.isFreeTier).toBe(true);
    expect(result.current.isBudgetExhausted).toBe(false);
    expect(result.current.shouldShowBudgetCompletedMessage).toBe(true);
    expect(result.current.isAtLimit).toBe(false);
    expect(result.current.usagePct).toBe(0);
  });

  it('treats paid users with no remaining recurring budget as exhausted', async () => {
    const { useUsageState } = await import('./useUsageState');
    mockGetCurrentPlan.mockResolvedValue(basicPlan());
    mockGetTeamUsage.mockResolvedValue(buildUsage({ remainingUsd: 0, cycleBudgetUsd: 10 }));

    const { result } = renderHook(() => useUsageState());

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
    });

    expect(result.current.isBudgetExhausted).toBe(true);
    expect(result.current.shouldShowBudgetCompletedMessage).toBe(true);
    expect(result.current.isAtLimit).toBe(true);
    expect(result.current.usagePct).toBe(1);
  });

  it('does not show the completed-budget message when credits remain without a recurring budget', async () => {
    const { useUsageState } = await import('./useUsageState');
    mockGetCurrentPlan.mockResolvedValue(freePlan());
    mockGetTeamUsage.mockResolvedValue(buildUsage({ remainingUsd: 7, cycleBudgetUsd: 0 }));

    const { result } = renderHook(() => useUsageState());

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
    });

    expect(result.current.isBudgetExhausted).toBe(false);
    expect(result.current.shouldShowBudgetCompletedMessage).toBe(false);
  });

  it('swallows CoreRpcError(kind=auth_expired) so it cannot leak to window.unhandledrejection (#1472)', async () => {
    const { useUsageState } = await import('./useUsageState');
    const { CoreRpcError } = await import('../services/coreRpcClient');

    mockGetCurrentPlan.mockResolvedValue(freePlan());
    mockGetTeamUsage.mockRejectedValue(
      new CoreRpcError(
        'GET /teams failed (401 Unauthorized): Session expired. Please log in again.',
        'auth_expired',
        401
      )
    );

    const unhandled = vi.fn();
    window.addEventListener('unhandledrejection', unhandled);
    try {
      const { result } = renderHook(() => useUsageState());
      await waitFor(() => {
        expect(result.current.isLoading).toBe(false);
      });
      expect(result.current.teamUsage).toBeNull();
      expect(unhandled).not.toHaveBeenCalled();
    } finally {
      window.removeEventListener('unhandledrejection', unhandled);
    }
  });

  it('swallows non-auth transport errors silently (does not throw past Promise.all)', async () => {
    const { useUsageState } = await import('./useUsageState');
    mockGetCurrentPlan.mockResolvedValue(freePlan());
    mockGetTeamUsage.mockRejectedValue(new Error('ECONNREFUSED 127.0.0.1:7788'));

    const unhandled = vi.fn();
    window.addEventListener('unhandledrejection', unhandled);
    try {
      const { result } = renderHook(() => useUsageState());
      await waitFor(() => {
        expect(result.current.isLoading).toBe(false);
      });
      expect(result.current.teamUsage).toBeNull();
      expect(unhandled).not.toHaveBeenCalled();
    } finally {
      window.removeEventListener('unhandledrejection', unhandled);
    }
  });

  it('refetches when a global usage refresh is requested', async () => {
    const { useUsageState } = await import('./useUsageState');
    const { requestUsageRefresh } = await import('./usageRefresh');

    mockGetCurrentPlan.mockResolvedValue(basicPlan());
    mockGetTeamUsage
      .mockResolvedValueOnce(buildUsage({ remainingUsd: 9, cycleBudgetUsd: 10 }))
      .mockResolvedValueOnce(buildUsage({ remainingUsd: 7, cycleBudgetUsd: 10 }));

    const { result } = renderHook(() => useUsageState());

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
    });
    expect(result.current.teamUsage?.remainingUsd).toBe(9);

    act(() => {
      requestUsageRefresh();
    });

    await waitFor(() => {
      expect(result.current.teamUsage?.remainingUsd).toBe(7);
    });
  });
});
