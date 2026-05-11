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

describe('useUsageState', () => {
  beforeEach(() => {
    vi.resetModules();
    mockGetCurrentPlan.mockReset();
    mockGetTeamUsage.mockReset();
  });

  it('does not treat free users with zero recurring budget as exhausted', async () => {
    const { useUsageState } = await import('./useUsageState');

    mockGetCurrentPlan.mockResolvedValue({
      plan: 'FREE',
      hasActiveSubscription: false,
      planExpiry: null,
      subscription: null,
      monthlyBudgetUsd: 0,
      weeklyBudgetUsd: 0,
      fiveHourCapUsd: 0,
    });
    mockGetTeamUsage.mockResolvedValue({
      remainingUsd: 0,
      cycleBudgetUsd: 0,
      cycleLimit5hr: 0,
      cycleLimit7day: 0,
      fiveHourCapUsd: 0,
      fiveHourResetsAt: null,
      cycleStartDate: '2026-04-09T00:00:00.000Z',
      cycleEndsAt: '2026-04-16T00:00:00.000Z',
      bypassCycleLimit: false,
    });

    const { result } = renderHook(() => useUsageState());

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
    });

    expect(result.current.isFreeTier).toBe(true);
    expect(result.current.isBudgetExhausted).toBe(false);
    expect(result.current.shouldShowBudgetCompletedMessage).toBe(true);
    expect(result.current.isRateLimited).toBe(false);
    expect(result.current.isAtLimit).toBe(false);
    expect(result.current.usagePct7d).toBe(0);
  });

  it('treats paid users with no remaining recurring budget as exhausted', async () => {
    const { useUsageState } = await import('./useUsageState');

    mockGetCurrentPlan.mockResolvedValue({
      plan: 'BASIC',
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
      fiveHourCapUsd: 3,
    });
    mockGetTeamUsage.mockResolvedValue({
      remainingUsd: 0,
      cycleBudgetUsd: 10,
      cycleLimit5hr: 1,
      cycleLimit7day: 10,
      fiveHourCapUsd: 3,
      fiveHourResetsAt: null,
      cycleStartDate: '2026-04-09T00:00:00.000Z',
      cycleEndsAt: '2026-04-16T00:00:00.000Z',
      bypassCycleLimit: false,
    });

    const { result } = renderHook(() => useUsageState());

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
    });

    expect(result.current.isBudgetExhausted).toBe(true);
    expect(result.current.shouldShowBudgetCompletedMessage).toBe(true);
    expect(result.current.isAtLimit).toBe(true);
    expect(result.current.usagePct7d).toBe(1);
  });

  it('does not show the completed-budget message when credits remain without a recurring budget', async () => {
    const { useUsageState } = await import('./useUsageState');

    mockGetCurrentPlan.mockResolvedValue({
      plan: 'FREE',
      hasActiveSubscription: false,
      planExpiry: null,
      subscription: null,
      monthlyBudgetUsd: 0,
      weeklyBudgetUsd: 0,
      fiveHourCapUsd: 0,
    });
    mockGetTeamUsage.mockResolvedValue({
      remainingUsd: 7,
      cycleBudgetUsd: 0,
      cycleLimit5hr: 0,
      cycleLimit7day: 0,
      fiveHourCapUsd: 0,
      fiveHourResetsAt: null,
      cycleStartDate: '2026-04-09T00:00:00.000Z',
      cycleEndsAt: '2026-04-16T00:00:00.000Z',
      bypassCycleLimit: false,
    });

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

    mockGetCurrentPlan.mockResolvedValue({
      plan: 'FREE',
      hasActiveSubscription: false,
      planExpiry: null,
      subscription: null,
      monthlyBudgetUsd: 0,
      weeklyBudgetUsd: 0,
      fiveHourCapUsd: 0,
    });
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
      // Hook must NOT have a teamUsage value (auth-expired leg fell back to
      // null) and the rejection must NOT have surfaced as unhandled.
      expect(result.current.teamUsage).toBeNull();
      expect(unhandled).not.toHaveBeenCalled();
    } finally {
      window.removeEventListener('unhandledrejection', unhandled);
    }
  });

  it('swallows non-auth transport errors silently (does not throw past Promise.all)', async () => {
    const { useUsageState } = await import('./useUsageState');

    mockGetCurrentPlan.mockResolvedValue({
      plan: 'FREE',
      hasActiveSubscription: false,
      planExpiry: null,
      subscription: null,
      monthlyBudgetUsd: 0,
      weeklyBudgetUsd: 0,
      fiveHourCapUsd: 0,
    });
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

    mockGetCurrentPlan.mockResolvedValue({
      plan: 'BASIC',
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
      fiveHourCapUsd: 3,
    });
    mockGetTeamUsage
      .mockResolvedValueOnce({
        remainingUsd: 9,
        cycleBudgetUsd: 10,
        cycleLimit5hr: 1,
        cycleLimit7day: 1,
        fiveHourCapUsd: 3,
        fiveHourResetsAt: null,
        cycleStartDate: '2026-04-09T00:00:00.000Z',
        cycleEndsAt: '2026-04-16T00:00:00.000Z',
        bypassCycleLimit: false,
      })
      .mockResolvedValueOnce({
        remainingUsd: 7,
        cycleBudgetUsd: 10,
        cycleLimit5hr: 2,
        cycleLimit7day: 3,
        fiveHourCapUsd: 3,
        fiveHourResetsAt: null,
        cycleStartDate: '2026-04-09T00:00:00.000Z',
        cycleEndsAt: '2026-04-16T00:00:00.000Z',
        bypassCycleLimit: false,
      });

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
