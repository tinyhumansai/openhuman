import { configureStore } from '@reduxjs/toolkit';
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Provider } from 'react-redux';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import userErrorsReducer, { reportUserError } from '../../../store/userErrorsSlice';
import type { UserErrorDescriptor } from '../../../types/userError';
import NoticeCenter from '../NoticeCenter';
import { __resetNativeNotificationLatchForTests } from '../useEmbeddingBudgetNativeNotice';

const budgetState = vi.hoisted(() => ({ level: 'none' as string, pct: 0 }));
const usageState = vi.hoisted(() => ({
  teamUsage: null as unknown,
  isLoading: false,
  isAtLimit: false,
  isNearLimit: false,
  isFreeTier: false,
  usagePct: 0,
  shouldShowBudgetCompletedMessage: false,
}));
const applyOpenRouterFreeModels = vi.hoisted(() => vi.fn());
const showNativeNotification = vi.hoisted(() => vi.fn());

vi.mock('../../../hooks/useEmbeddingBudgetState', () => ({
  useEmbeddingBudgetState: () => budgetState,
}));
vi.mock('../../../hooks/useUsageState', () => ({ useUsageState: () => usageState }));
// openhuman#5820: the quarantine poll needs CoreStateProvider; not under test here.
vi.mock('../useMemoryQuarantinePoll', () => ({ useMemoryQuarantinePoll: () => undefined }));
vi.mock('../../../lib/nativeNotifications/tauriBridge', () => ({ showNativeNotification }));
vi.mock('../../../utils/openUrl', () => ({ openUrl: vi.fn() }));
vi.mock('../../../services/api/openrouterFreeModels', () => ({ applyOpenRouterFreeModels }));

const memoryError: UserErrorDescriptor = {
  id: 'memory_budget_exhausted:memory:managed',
  kind: 'memory_budget_exhausted',
  severity: 'error',
  scope: 'memory',
  titleKey: 'userErrors.memoryBudgetExhausted.title',
  bodyKey: 'userErrors.memoryBudgetExhausted.body',
  action: 'open_embeddings_settings',
};

function LocationProbe() {
  const location = useLocation();
  return <div data-testid="pathname">{`${location.pathname}${location.search}`}</div>;
}

function renderCenter(descriptors: UserErrorDescriptor[] = []) {
  const store = configureStore({ reducer: { userErrors: userErrorsReducer } });
  descriptors.forEach((d, i) => store.dispatch(reportUserError({ descriptor: d, at: 1000 + i })));
  // A FRESH element each time: React bails out of re-rendering when handed a
  // referentially identical one, so reusing a single `tree` constant would make
  // `rerender()` a no-op and hide a genuine change in the mocked hook state.
  const tree = () => (
    <Provider store={store}>
      <MemoryRouter initialEntries={['/chat']}>
        <NoticeCenter />
        <LocationProbe />
      </MemoryRouter>
    </Provider>
  );
  const utils = render(tree());
  return { store, ...utils, rerender: () => utils.rerender(tree()) };
}

describe('NoticeCenter', () => {
  beforeEach(() => {
    budgetState.level = 'none';
    budgetState.pct = 0;
    usageState.teamUsage = null;
    usageState.isAtLimit = false;
    usageState.isNearLimit = false;
    usageState.isFreeTier = false;
    usageState.usagePct = 0;
    usageState.shouldShowBudgetCompletedMessage = false;
    showNativeNotification.mockClear();
    applyOpenRouterFreeModels.mockReset();
    applyOpenRouterFreeModels.mockResolvedValue(undefined);
    // Module-scoped once-per-session latch — reset so each test starts cold.
    __resetNativeNotificationLatchForTests();
  });

  it('renders no chrome at all when there is nothing to say', () => {
    renderCenter([]);
    expect(screen.queryByTestId('notice-center')).toBeNull();
  });

  it('anchors to the bottom-right corner', () => {
    renderCenter([memoryError]);

    const center = screen.getByTestId('notice-center');
    expect(center.className).toContain('bottom-2');
    expect(center.className).toContain('right-2');
    expect(center.className).not.toContain('left-2');
  });

  it('badges the active count and opens the panel on click', async () => {
    renderCenter([memoryError]);

    expect(screen.getByTestId('notice-badge')).toHaveTextContent('1');
    expect(screen.queryByTestId('notice-panel')).toBeNull();

    await userEvent.click(screen.getByTestId('notice-trigger'));

    const panel = screen.getByTestId('notice-panel');
    expect(within(panel).getByText('Memory has stopped growing')).toBeInTheDocument();
  });

  /**
   * The whole point of the consolidation: the memory-embedding warning used to
   * be a full-width banner above every route. It is a notice now, and it has to
   * reach this panel from the budget hook, not just from the errors slice.
   */
  it('raises the memory-embedding budget state as a notice', async () => {
    budgetState.level = 'exhausted';
    budgetState.pct = 100;
    renderCenter([]);

    await userEvent.click(screen.getByTestId('notice-trigger'));

    expect(screen.getByText('Memory has stopped growing')).toBeInTheDocument();
    expect(screen.getByTestId('notice-action')).toHaveTextContent('Set up embeddings');
  });

  it('routes to the embeddings screen from the memory notice action', async () => {
    budgetState.level = 'exhausted';
    renderCenter([]);

    await userEvent.click(screen.getByTestId('notice-trigger'));
    await userEvent.click(screen.getByTestId('notice-action'));

    expect(screen.getByTestId('pathname')).toHaveTextContent('/connections?tab=embeddings');
  });

  it('lets the early budget warning be dismissed but not the exhausted state', async () => {
    budgetState.level = 'warn';
    budgetState.pct = 80;
    const { rerender } = renderCenter([]);

    await userEvent.click(screen.getByTestId('notice-trigger'));
    expect(screen.getByTestId('notice-dismiss')).toBeInTheDocument();
    await userEvent.click(screen.getByTestId('notice-dismiss'));
    expect(screen.queryByTestId('notice-center')).toBeNull();

    // The escalation is a different notice id, so silencing the 75% warning
    // must not silence it — otherwise the user is back to a silent failure.
    budgetState.level = 'exhausted';
    rerender();
    expect(screen.getByTestId('notice-center')).toBeInTheDocument();
  });

  it('raises the usage limit as a notice that cannot be dismissed at the limit', async () => {
    usageState.teamUsage = { plan: 'free' };
    usageState.isAtLimit = true;
    renderCenter([]);

    await userEvent.click(screen.getByTestId('notice-trigger'));

    expect(screen.getByText('Usage limit reached')).toBeInTheDocument();
    // Already gated — silencing it would hide the only explanation.
    expect(screen.queryByTestId('notice-dismiss')).toBeNull();
  });

  it('sorts errors above warnings so the badge summarises the worst state', async () => {
    budgetState.level = 'warn';
    usageState.teamUsage = { plan: 'free' };
    usageState.isAtLimit = true;
    renderCenter([]);

    await userEvent.click(screen.getByTestId('notice-trigger'));

    const titles = screen.getAllByTestId('notice-item').map(item => item.textContent ?? '');
    expect(titles[0]).toContain('Usage limit reached');
    expect(screen.getByTestId('notice-badge').className).toContain('bg-coral-500');
  });

  /**
   * This state was rendered in four places off one flag (both Conversations
   * layouts, the new-window hero, Home). It is one notice now, and it has to
   * carry BOTH remediations the banner offered — the OpenRouter one is the
   * only fix that does not require a payment method.
   */
  it('raises the spent cycle budget with both remediations', async () => {
    usageState.teamUsage = { cycleBudgetUsd: 5, cycleEndsAt: null };
    usageState.shouldShowBudgetCompletedMessage = true;
    renderCenter([]);

    await userEvent.click(screen.getByTestId('notice-trigger'));

    expect(screen.getByText(/used your included cycle budget/i)).toBeInTheDocument();
    expect(screen.getByTestId('notice-action')).toHaveTextContent('Top Up');
    expect(screen.getByTestId('notice-secondary-action')).toHaveTextContent(
      'Use OpenRouter free models'
    );
  });

  it('runs the OpenRouter routing helper from the secondary action', async () => {
    usageState.teamUsage = { cycleBudgetUsd: 5, cycleEndsAt: null };
    usageState.shouldShowBudgetCompletedMessage = true;
    renderCenter([]);

    await userEvent.click(screen.getByTestId('notice-trigger'));
    await userEvent.click(screen.getByTestId('notice-secondary-action'));

    expect(applyOpenRouterFreeModels).toHaveBeenCalledTimes(1);
  });

  it('surfaces a failed OpenRouter switch as its own notice', async () => {
    applyOpenRouterFreeModels.mockRejectedValueOnce(new Error('nope'));
    usageState.teamUsage = { cycleBudgetUsd: 0, cycleEndsAt: null };
    usageState.shouldShowBudgetCompletedMessage = true;
    renderCenter([]);

    await userEvent.click(screen.getByTestId('notice-trigger'));
    await userEvent.click(screen.getByTestId('notice-secondary-action'));

    // The banner this replaced showed the failure inline; without this the
    // click would look like it silently did nothing.
    expect(await screen.findByText(/couldn't|could not|failed/i)).toBeInTheDocument();
  });

  it('raises an integration outage with the source detail verbatim', async () => {
    renderCenter([
      {
        id: 'integration_degraded:integration:composio',
        kind: 'integration_degraded',
        severity: 'warning',
        scope: 'integration',
        provider: 'composio',
        titleKey: 'userErrors.integrationDegraded.title',
        bodyKey: 'userErrors.integrationDegraded.body',
        detail: '[composio] list_connections failed: credits are exhausted.',
        action: 'open_connections',
      },
    ]);

    await userEvent.click(screen.getByTestId('notice-trigger'));

    expect(screen.getByText(/Connections are showing stale status/i)).toBeInTheDocument();
    // The backend's own user-facing text says what actually failed — the
    // translated body cannot.
    expect(screen.getByText(/list_connections failed/i)).toBeInTheDocument();

    await userEvent.click(screen.getByTestId('notice-action'));
    expect(screen.getByTestId('pathname')).toHaveTextContent('/connections?tab=skills');
  });

  it('raises the near-limit warning and persists its dismissal for 24h', async () => {
    usageState.teamUsage = { plan: 'free' };
    usageState.isNearLimit = true;
    usageState.isFreeTier = true;
    usageState.usagePct = 0.85;
    localStorage.removeItem('openhuman:upsell:conversations-warning');
    renderCenter([]);

    await userEvent.click(screen.getByTestId('notice-trigger'));
    expect(screen.getByText('Approaching usage limit')).toBeInTheDocument();

    await userEvent.click(screen.getByTestId('notice-dismiss'));

    // Persisted, not session-only: usage creeps up over days, so re-nagging on
    // every restart is exactly what the banner's 24h cooldown prevented.
    expect(localStorage.getItem('openhuman:upsell:conversations-warning')).not.toBeNull();
    expect(screen.queryByTestId('notice-center')).toBeNull();
  });

  it('dismisses a classified error out of the list', async () => {
    renderCenter([memoryError]);

    await userEvent.click(screen.getByTestId('notice-trigger'));
    await userEvent.click(screen.getByTestId('notice-dismiss'));

    expect(screen.queryByTestId('notice-center')).toBeNull();
  });

  it('closes on Escape', async () => {
    renderCenter([memoryError]);

    await userEvent.click(screen.getByTestId('notice-trigger'));
    expect(screen.getByTestId('notice-panel')).toBeInTheDocument();

    await userEvent.keyboard('{Escape}');
    expect(screen.queryByTestId('notice-panel')).toBeNull();
  });

  it('fires the OS notification once when the memory budget is exhausted', () => {
    budgetState.level = 'exhausted';
    renderCenter([]);

    expect(showNativeNotification).toHaveBeenCalledTimes(1);
    expect(showNativeNotification.mock.calls[0][0]).toMatchObject({
      tag: 'memory-embedding-budget-exhausted',
    });
  });
});
