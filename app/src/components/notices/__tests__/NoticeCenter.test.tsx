import { configureStore } from '@reduxjs/toolkit';
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Provider } from 'react-redux';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import userErrorsReducer, { reportUserError } from '../../../store/userErrorsSlice';
import type { UserErrorDescriptor } from '../../../types/userError';
import NoticeCenter from '../NoticeCenter';

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

vi.mock('../../../hooks/useUsageState', () => ({ useUsageState: () => usageState }));
vi.mock('../../../utils/openUrl', () => ({ openUrl: vi.fn() }));
vi.mock('../../../services/api/openrouterFreeModels', () => ({ applyOpenRouterFreeModels }));

const keyError: UserErrorDescriptor = {
  id: 'api_key_missing:provider:openrouter',
  kind: 'api_key_missing',
  severity: 'error',
  scope: 'provider',
  titleKey: 'userErrors.apiKeyMissing.title',
  bodyKey: 'userErrors.apiKeyMissing.body',
  action: 'open_provider_settings',
};

const modelWarning: UserErrorDescriptor = {
  id: 'local_model_unavailable:chat',
  kind: 'local_model_unavailable',
  severity: 'warning',
  scope: 'chat',
  titleKey: 'userErrors.localModelUnavailable.title',
  bodyKey: 'userErrors.localModelUnavailable.body',
  action: 'dismiss',
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
    usageState.teamUsage = null;
    usageState.isAtLimit = false;
    usageState.isNearLimit = false;
    usageState.isFreeTier = false;
    usageState.usagePct = 0;
    usageState.shouldShowBudgetCompletedMessage = false;
    applyOpenRouterFreeModels.mockReset();
    applyOpenRouterFreeModels.mockResolvedValue(undefined);
  });

  it('renders no chrome at all when there is nothing to say', () => {
    renderCenter([]);
    expect(screen.queryByTestId('notice-center')).toBeNull();
  });

  it('anchors to the bottom-right corner', () => {
    renderCenter([keyError]);

    const center = screen.getByTestId('notice-center');
    expect(center.className).toContain('bottom-2');
    expect(center.className).toContain('right-2');
    expect(center.className).not.toContain('left-2');
  });

  it('badges the active count and opens the panel on click', async () => {
    renderCenter([keyError]);

    expect(screen.getByTestId('notice-badge')).toHaveTextContent('1');
    expect(screen.queryByTestId('notice-panel')).toBeNull();

    await userEvent.click(screen.getByTestId('notice-trigger'));

    const panel = screen.getByTestId('notice-panel');
    expect(within(panel).getByText('API key required')).toBeInTheDocument();
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
    usageState.teamUsage = { plan: 'free' };
    usageState.isAtLimit = true;
    renderCenter([modelWarning]);

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
    renderCenter([keyError]);

    await userEvent.click(screen.getByTestId('notice-trigger'));
    await userEvent.click(screen.getByTestId('notice-dismiss'));

    expect(screen.queryByTestId('notice-center')).toBeNull();
  });

  it('closes on Escape', async () => {
    renderCenter([keyError]);

    await userEvent.click(screen.getByTestId('notice-trigger'));
    expect(screen.getByTestId('notice-panel')).toBeInTheDocument();

    await userEvent.keyboard('{Escape}');
    expect(screen.queryByTestId('notice-panel')).toBeNull();
  });
});
