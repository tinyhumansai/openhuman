import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const mockGetCurrentPlan = vi.fn();
const mockGetTeamUsage = vi.fn();
const mockLoadAISettings = vi.fn();

vi.mock('../services/api/billingApi', () => ({
  billingApi: { getCurrentPlan: () => mockGetCurrentPlan() },
}));

vi.mock('../services/api/creditsApi', () => ({
  creditsApi: { getTeamUsage: () => mockGetTeamUsage() },
}));

vi.mock('../services/api/aiSettingsApi', async () => {
  const actual = await vi.importActual<typeof import('../services/api/aiSettingsApi')>(
    '../services/api/aiSettingsApi'
  );
  return { ...actual, loadAISettings: () => mockLoadAISettings() };
});

// useUsageState gates polling on auth (#3297). Default authenticated so every
// existing budget-gating assertion keeps exercising the fetch path; the gating
// test below flips it false.
const { mockAuthState } = vi.hoisted(() => ({ mockAuthState: { isAuthenticated: true } }));
vi.mock('../providers/CoreStateProvider', () => ({
  useCoreState: () => ({ snapshot: { auth: { isAuthenticated: mockAuthState.isAuthenticated } } }),
}));

// All chat workloads routed to OpenHuman — the default for every existing
// test case (matches the legacy "you have a hosted-backend budget" world).
const ALL_OPENHUMAN_AI_SETTINGS = {
  cloudProviders: [],
  routing: {
    chat: { kind: 'openhuman' as const },
    reasoning: { kind: 'openhuman' as const },
    agentic: { kind: 'openhuman' as const },
    coding: { kind: 'openhuman' as const },
    memory: { kind: 'openhuman' as const },
    embeddings: { kind: 'openhuman' as const },
    heartbeat: { kind: 'openhuman' as const },
    learning: { kind: 'openhuman' as const },
    subconscious: { kind: 'openhuman' as const },
  },
};

const ALL_LOCAL_AI_SETTINGS = {
  cloudProviders: [],
  routing: {
    chat: { kind: 'local' as const, model: 'qwen3:8b' },
    reasoning: { kind: 'local' as const, model: 'qwen3:8b' },
    agentic: { kind: 'local' as const, model: 'qwen3:8b' },
    coding: { kind: 'local' as const, model: 'qwen3:8b' },
    memory: { kind: 'local' as const, model: 'nomic-embed-text' },
    embeddings: { kind: 'local' as const, model: 'nomic-embed-text' },
    heartbeat: { kind: 'local' as const, model: 'qwen3:8b' },
    learning: { kind: 'local' as const, model: 'qwen3:8b' },
    subconscious: { kind: 'local' as const, model: 'qwen3:8b' },
  },
};

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
    mockLoadAISettings.mockReset();
    // Default authenticated; the auth-gating test opts out explicitly.
    mockAuthState.isAuthenticated = true;
    // Default: keep the OpenHuman-routed world so every legacy assertion
    // about budget gating stays identical until a test opts into the
    // routed-away scenarios below.
    mockLoadAISettings.mockResolvedValue(ALL_OPENHUMAN_AI_SETTINGS);
  });

  it('does not show the completed-budget message for free users with zero recurring budget', async () => {
    const { useUsageState } = await import('./useUsageState');
    mockGetCurrentPlan.mockResolvedValue(freePlan());
    mockGetTeamUsage.mockResolvedValue(buildUsage());

    const { result } = renderHook(() => useUsageState());

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
    });

    expect(result.current.isFreeTier).toBe(true);
    expect(result.current.isBudgetExhausted).toBe(false);
    expect(result.current.shouldShowBudgetCompletedMessage).toBe(false);
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

  // -- #2040 / #2041 — budget banner is suppressed when chat routes away ----

  it('suppresses the budget banner when every chat workload routes to a user-supplied cloud provider (#2040, #2041)', async () => {
    const { useUsageState } = await import('./useUsageState');

    // Plan + usage say "budget exhausted" — but the user has saved an
    // OpenRouter key and routed reasoning/agentic/coding away from
    // openhuman. The banner that previously said "Your included budget is
    // complete" should NOT show, because the user is paying OpenRouter,
    // not OpenHuman, for chat inference.
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
      cycleLimit5hr: 3, // at-the-cap to also exercise the rate-limit gate
      cycleLimit7day: 10,
      fiveHourCapUsd: 3,
      fiveHourResetsAt: null,
      cycleStartDate: '2026-04-09T00:00:00.000Z',
      cycleEndsAt: '2026-04-16T00:00:00.000Z',
      bypassCycleLimit: false,
    });
    mockLoadAISettings.mockResolvedValue({
      cloudProviders: [],
      routing: {
        chat: { kind: 'cloud', providerSlug: 'openrouter', model: 'anthropic/claude-sonnet-4.6' },
        reasoning: {
          kind: 'cloud',
          providerSlug: 'openrouter',
          model: 'google/gemini-3-flash-preview',
        },
        agentic: {
          kind: 'cloud',
          providerSlug: 'openrouter',
          model: 'anthropic/claude-sonnet-4.6',
        },
        coding: { kind: 'cloud', providerSlug: 'openrouter', model: 'anthropic/claude-sonnet-4.6' },
        // Background workloads may still route to openhuman — the suppression
        // logic only consults CHAT_WORKLOADS (chat/reasoning/agentic/coding).
        memory: { kind: 'openhuman' },
        embeddings: { kind: 'openhuman' },
        heartbeat: { kind: 'openhuman' },
        learning: { kind: 'openhuman' },
        subconscious: { kind: 'openhuman' },
      },
    });

    const { result } = renderHook(() => useUsageState());

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
    });

    expect(result.current.isFullyRoutedAway).toBe(true);
    expect(result.current.shouldShowBudgetCompletedMessage).toBe(false);
    expect(result.current.isBudgetExhausted).toBe(false);
    expect(result.current.isAtLimit).toBe(false);
  });

  // -- #3767 — authoritative core-side BYO-key bypass flag --------------------

  it('suppresses the budget banner when the core reports creditsBypass for the active mode (#3767)', async () => {
    const { useUsageState } = await import('./useUsageState');

    // Budget exhausted, and the raw routing strings still read as managed
    // (kind=openhuman). The core reports the chat tier runs on a usable BYO
    // provider (creditsBypass.chat=true), which must win for the default mode.
    mockGetCurrentPlan.mockResolvedValue(basicPlan());
    mockGetTeamUsage.mockResolvedValue(buildUsage({ remainingUsd: 0, cycleBudgetUsd: 10 }));
    mockLoadAISettings.mockResolvedValue({
      ...ALL_OPENHUMAN_AI_SETTINGS,
      creditsBypass: { chat: true, reasoning: true },
    });

    const { result } = renderHook(() => useUsageState());

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
    });

    expect(result.current.isFullyRoutedAway).toBe(true);
    expect(result.current.shouldShowBudgetCompletedMessage).toBe(false);
    expect(result.current.isBudgetExhausted).toBe(false);
    expect(result.current.isAtLimit).toBe(false);
  });

  it('still shows the budget banner when creditsBypass is false for the active mode (#3767)', async () => {
    const { useUsageState } = await import('./useUsageState');

    // Neither chat-mode tier on a usable BYO provider → gate stays on.
    mockGetCurrentPlan.mockResolvedValue(basicPlan());
    mockGetTeamUsage.mockResolvedValue(buildUsage({ remainingUsd: 0, cycleBudgetUsd: 10 }));
    mockLoadAISettings.mockResolvedValue({
      ...ALL_OPENHUMAN_AI_SETTINGS,
      creditsBypass: { chat: false, reasoning: false },
    });

    const { result } = renderHook(() => useUsageState());

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
    });

    expect(result.current.isFullyRoutedAway).toBe(false);
    expect(result.current.shouldShowBudgetCompletedMessage).toBe(true);
    expect(result.current.isBudgetExhausted).toBe(true);
    expect(result.current.isAtLimit).toBe(true);
  });

  it('gates per selected chat mode — Quick bypassed, Reasoning gated when only chat is BYO (#3767)', async () => {
    const { useUsageState } = await import('./useUsageState');

    // chat tier on BYO, reasoning tier still managed. Quick mode (chat) should
    // bypass; Reasoning mode (reasoning) should stay gated.
    mockGetCurrentPlan.mockResolvedValue(basicPlan());
    mockGetTeamUsage.mockResolvedValue(buildUsage({ remainingUsd: 0, cycleBudgetUsd: 10 }));
    mockLoadAISettings.mockResolvedValue({
      ...ALL_OPENHUMAN_AI_SETTINGS,
      creditsBypass: { chat: true, reasoning: false },
    });

    const quick = renderHook(() => useUsageState('chat'));
    await waitFor(() => expect(quick.result.current.isLoading).toBe(false));
    expect(quick.result.current.isAtLimit).toBe(false);
    expect(quick.result.current.shouldShowBudgetCompletedMessage).toBe(false);

    const reasoning = renderHook(() => useUsageState('reasoning'));
    await waitFor(() => expect(reasoning.result.current.isLoading).toBe(false));
    expect(reasoning.result.current.isAtLimit).toBe(true);
    expect(reasoning.result.current.shouldShowBudgetCompletedMessage).toBe(true);
  });

  it('still shows the budget banner when at least one chat workload remains on OpenHuman', async () => {
    const { useUsageState } = await import('./useUsageState');

    // User has saved an OpenRouter key for agentic+coding but left reasoning
    // on openhuman — they're still partially dependent on the included
    // budget, so the banner must keep showing.
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
    mockLoadAISettings.mockResolvedValue({
      cloudProviders: [],
      routing: {
        reasoning: { kind: 'openhuman' }, // still on the hosted backend
        agentic: {
          kind: 'cloud',
          providerSlug: 'openrouter',
          model: 'anthropic/claude-sonnet-4.6',
        },
        coding: { kind: 'cloud', providerSlug: 'openrouter', model: 'anthropic/claude-sonnet-4.6' },
        memory: { kind: 'openhuman' },
        embeddings: { kind: 'openhuman' },
        heartbeat: { kind: 'openhuman' },
        learning: { kind: 'openhuman' },
        subconscious: { kind: 'openhuman' },
      },
    });

    const { result } = renderHook(() => useUsageState());

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
    });

    expect(result.current.isFullyRoutedAway).toBe(false);
    expect(result.current.shouldShowBudgetCompletedMessage).toBe(true);
    expect(result.current.isBudgetExhausted).toBe(true);
    expect(result.current.isAtLimit).toBe(true);
  });

  it('treats missing aiSettings (fetch failure) as conservative — banner still shows when budget is otherwise exhausted', async () => {
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
    // Simulate a transient failure in the AI-settings fetch — the budget
    // gate must NOT silently disable itself just because we couldn't read
    // the routing config.
    mockLoadAISettings.mockRejectedValue(new Error('network down'));

    const { result } = renderHook(() => useUsageState());

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
    });

    expect(result.current.isFullyRoutedAway).toBe(false);
    expect(result.current.shouldShowBudgetCompletedMessage).toBe(true);
    expect(result.current.isBudgetExhausted).toBe(true);
    expect(result.current.isAtLimit).toBe(true);
  });

  it('suppresses the budget banner when every chat workload routes to a local Ollama provider (#2040, #2041, ProviderRef kind=local)', async () => {
    // Companion to the cloud-provider case above. The PR description claims
    // suppression also applies to ProviderRef kind='local' (Ollama on-device
    // inference). graycyrus review on #2053 flagged that none of the new
    // tests pinned that path. This one does.
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
      cycleLimit5hr: 3, // at the cap to also exercise rate-limit gating
      cycleLimit7day: 10,
      fiveHourCapUsd: 3,
      fiveHourResetsAt: null,
      cycleStartDate: '2026-04-09T00:00:00.000Z',
      cycleEndsAt: '2026-04-16T00:00:00.000Z',
      bypassCycleLimit: false,
    });
    mockLoadAISettings.mockResolvedValue({
      cloudProviders: [],
      routing: {
        // All chat workloads on local Ollama models.
        chat: { kind: 'local', model: 'qwen3:8b' },
        reasoning: { kind: 'local', model: 'qwen3:8b' },
        agentic: { kind: 'local', model: 'qwen3:8b' },
        coding: { kind: 'local', model: 'qwen3:8b' },
        // Background workloads are intentionally left on openhuman to
        // prove the gate is keyed on chat workloads only.
        memory: { kind: 'openhuman' },
        embeddings: { kind: 'openhuman' },
        heartbeat: { kind: 'openhuman' },
        learning: { kind: 'openhuman' },
        subconscious: { kind: 'openhuman' },
      },
    });

    const { result } = renderHook(() => useUsageState());

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
    });

    expect(result.current.isFullyRoutedAway).toBe(true);
    expect(result.current.shouldShowBudgetCompletedMessage).toBe(false);
    expect(result.current.isBudgetExhausted).toBe(false);
    expect(result.current.isAtLimit).toBe(false);
  });

  it('does not fetch billing usage when every workload routes away from OpenHuman (#2020)', async () => {
    const { useUsageState } = await import('./useUsageState');

    mockLoadAISettings.mockResolvedValue(ALL_LOCAL_AI_SETTINGS);
    mockGetCurrentPlan.mockRejectedValue(new Error('billing plan should not be fetched'));
    mockGetTeamUsage.mockRejectedValue(new Error('team usage should not be fetched'));

    const { result } = renderHook(() => useUsageState());

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
    });

    expect(result.current.teamUsage).toBeNull();
    expect(result.current.currentPlan).toBeNull();
    expect(result.current.isFullyRoutedAway).toBe(true);
    expect(mockGetCurrentPlan).not.toHaveBeenCalled();
    expect(mockGetTeamUsage).not.toHaveBeenCalled();
  });

  it('rechecks routing before returning a warm billing cache (#2020)', async () => {
    const { useUsageState } = await import('./useUsageState');

    mockGetCurrentPlan.mockResolvedValue(basicPlan());
    mockGetTeamUsage.mockResolvedValue(buildUsage({ remainingUsd: 0, cycleBudgetUsd: 10 }));
    mockLoadAISettings
      .mockResolvedValueOnce(ALL_OPENHUMAN_AI_SETTINGS)
      .mockResolvedValueOnce(ALL_LOCAL_AI_SETTINGS);

    const first = renderHook(() => useUsageState());
    await waitFor(() => {
      expect(first.result.current.isLoading).toBe(false);
    });
    expect(first.result.current.teamUsage).not.toBeNull();
    first.unmount();

    mockGetCurrentPlan.mockClear();
    mockGetTeamUsage.mockClear();

    const second = renderHook(() => useUsageState());
    await waitFor(() => {
      expect(second.result.current.isLoading).toBe(false);
    });

    expect(second.result.current.teamUsage).toBeNull();
    expect(second.result.current.currentPlan).toBeNull();
    expect(second.result.current.isFullyRoutedAway).toBe(true);
    expect(mockLoadAISettings).toHaveBeenCalledTimes(2);
    expect(mockGetCurrentPlan).not.toHaveBeenCalled();
    expect(mockGetTeamUsage).not.toHaveBeenCalled();
  });

  it('suppresses near-limit banner when chat is fully routed away but background workloads remain (#3097)', async () => {
    // Background workloads (memory, heartbeat, …) keep the billing API call
    // alive (ALL_WORKLOADS check), but isFullyRoutedAway (CHAT_WORKLOADS) is
    // true so the near-limit banner must NOT show — the user's chat is not
    // on OpenHuman's budget.
    const { useUsageState } = await import('./useUsageState');

    mockGetCurrentPlan.mockResolvedValue(freePlan());
    // Usage at 90% — would trigger isNearLimit for OpenHuman-routed users.
    mockGetTeamUsage.mockResolvedValue(buildUsage({ remainingUsd: 1, cycleBudgetUsd: 10 }));
    mockLoadAISettings.mockResolvedValue({
      cloudProviders: [],
      routing: {
        chat: { kind: 'local' as const, model: 'qwen3:8b' },
        reasoning: { kind: 'local' as const, model: 'qwen3:8b' },
        agentic: { kind: 'local' as const, model: 'qwen3:8b' },
        coding: { kind: 'local' as const, model: 'qwen3:8b' },
        // background workloads still on OpenHuman → billing API is still called
        memory: { kind: 'openhuman' as const },
        embeddings: { kind: 'openhuman' as const },
        heartbeat: { kind: 'openhuman' as const },
        learning: { kind: 'openhuman' as const },
        subconscious: { kind: 'openhuman' as const },
      },
    });

    const { result } = renderHook(() => useUsageState());

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
    });

    expect(result.current.isFullyRoutedAway).toBe(true);
    // usagePct ~90% but user is routed away from OpenHuman — no near-limit warning.
    expect(result.current.isNearLimit).toBe(false);
    expect(result.current.isAtLimit).toBe(false);
    // billing was still fetched because background workloads remain on OpenHuman
    expect(mockGetTeamUsage).toHaveBeenCalledTimes(1);
  });

  it('still fetches billing when a background workload remains on OpenHuman', async () => {
    const { useUsageState } = await import('./useUsageState');

    mockLoadAISettings.mockResolvedValue({
      ...ALL_LOCAL_AI_SETTINGS,
      routing: { ...ALL_LOCAL_AI_SETTINGS.routing, memory: { kind: 'openhuman' as const } },
    });
    mockGetCurrentPlan.mockResolvedValue(freePlan());
    mockGetTeamUsage.mockResolvedValue(buildUsage());

    const { result } = renderHook(() => useUsageState());

    await waitFor(() => {
      expect(result.current.isLoading).toBe(false);
    });

    expect(result.current.isFullyRoutedAway).toBe(true);
    expect(mockGetCurrentPlan).toHaveBeenCalledTimes(1);
    expect(mockGetTeamUsage).toHaveBeenCalledTimes(1);
  });

  it('rethrows CoreRpcError(kind=auth_expired) from loadAISettings instead of swallowing it (graycyrus review on #2053)', async () => {
    // The two sibling fetches (getTeamUsage, getCurrentPlan) explicitly
    // re-throw auth_expired so coreRpcClient's global re-auth event fires.
    // loadAISettings goes through the same RPC layer and must follow the
    // same contract — otherwise a session-expired user gets stale data
    // instead of a re-auth prompt.
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
    mockLoadAISettings.mockRejectedValue(
      new CoreRpcError(
        'GET /ai/settings failed (401 Unauthorized): Session expired.',
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
      // The hook's outer .catch swallows auth_expired silently (matching
      // the existing #1472 contract). The rejection must NOT have leaked
      // to window.unhandledrejection.
      expect(result.current.teamUsage).toBeNull();
      expect(result.current.currentPlan).toBeNull();
      expect(result.current.isFullyRoutedAway).toBe(false);
      expect(unhandled).not.toHaveBeenCalled();
    } finally {
      window.removeEventListener('unhandledrejection', unhandled);
    }
  });

  // -- #3297 — auth gating before dispatch (TAURI-RUST-8WY / 8WZ) -----------

  it('does not dispatch usage/plan RPCs while unauthenticated (#3297, TAURI-RUST-8WY/8WZ)', async () => {
    // Signed out (pre-login, or after a SessionExpired clear): these RPCs
    // require a backend session, so dispatching them is a guaranteed 401 at
    // the backend — the flood this fix removes. The hook must skip the fetch
    // entirely rather than round-trip to a doomed call.
    mockAuthState.isAuthenticated = false;
    const { useUsageState } = await import('./useUsageState');
    mockGetCurrentPlan.mockRejectedValue(new Error('plan must not be fetched while signed out'));
    mockGetTeamUsage.mockRejectedValue(new Error('usage must not be fetched while signed out'));
    mockLoadAISettings.mockRejectedValue(new Error('settings must not load while signed out'));

    const { result } = renderHook(() => useUsageState());
    // Let any (incorrectly-scheduled) async fetch microtasks flush.
    await Promise.resolve();
    await Promise.resolve();

    expect(mockLoadAISettings).not.toHaveBeenCalled();
    expect(mockGetTeamUsage).not.toHaveBeenCalled();
    expect(mockGetCurrentPlan).not.toHaveBeenCalled();
    expect(result.current.teamUsage).toBeNull();
    expect(result.current.currentPlan).toBeNull();
    expect(result.current.isLoading).toBe(false);
  });
});
