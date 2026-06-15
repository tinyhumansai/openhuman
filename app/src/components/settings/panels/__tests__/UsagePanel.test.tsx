import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { loadAISettings } from '../../../../services/api/aiSettingsApi';
import { renderWithProviders } from '../../../../test/test-utils';
import UsagePanel from '../UsagePanel';

// The tab bodies are heavy (chart pipeline, multi-RPC fetches) — stub them so
// these tests stay focused on the hash <-> tab mapping that UsagePanel owns.
vi.mock('../../../dashboard/CostDashboardPanel', () => ({
  default: ({ embedded }: { embedded?: boolean }) => (
    <div data-testid="stub-cost-dashboard" data-embedded={String(embedded ?? false)} />
  ),
}));

vi.mock('../AIPanel', () => ({
  BackgroundLoopControls: ({ view, hideHeader }: { view?: string; hideHeader?: boolean }) => (
    <div
      data-testid="stub-background-loops"
      data-view={view}
      data-hide-header={String(hideHeader ?? false)}
    />
  ),
}));

vi.mock('../../../../services/api/aiSettingsApi', async () => {
  const actual = await vi.importActual<typeof import('../../../../services/api/aiSettingsApi')>(
    '../../../../services/api/aiSettingsApi'
  );
  return { ...actual, loadAISettings: vi.fn() };
});

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: vi.fn(),
    navigateToSettings: vi.fn(),
    breadcrumbs: [],
  }),
}));

const mockLoad = vi.mocked(loadAISettings);

const snapshot = { routing: {}, cloudProviders: [] } as unknown as Awaited<
  ReturnType<typeof loadAISettings>
>;

describe('UsagePanel', () => {
  beforeEach(() => {
    mockLoad.mockReset();
    mockLoad.mockResolvedValue(snapshot);
  });

  test('default hash renders the Costs tab with the embedded cost dashboard', () => {
    renderWithProviders(<UsagePanel />, { initialEntries: ['/settings/usage'] });

    expect(screen.getByTestId('usage-tab-costs')).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByTestId('usage-tab-background')).toHaveAttribute('aria-selected', 'false');
    expect(screen.getByTestId('stub-cost-dashboard')).toHaveAttribute('data-embedded', 'true');
    // Costs tab must not pay for the AI-settings snapshot.
    expect(mockLoad).not.toHaveBeenCalled();
  });

  test('#background hash selects the Background tab and renders the loop controls', async () => {
    renderWithProviders(<UsagePanel />, { initialEntries: ['/settings/usage#background'] });

    expect(screen.getByTestId('usage-tab-background')).toHaveAttribute('aria-selected', 'true');
    expect(screen.queryByTestId('stub-cost-dashboard')).not.toBeInTheDocument();

    const controls = await screen.findByTestId('stub-background-loops');
    expect(controls).toHaveAttribute('data-view', 'all');
    expect(controls).toHaveAttribute('data-hide-header', 'true');
    expect(mockLoad).toHaveBeenCalledTimes(1);
  });

  test('clicking the Background tab switches the view in place', async () => {
    renderWithProviders(<UsagePanel />, { initialEntries: ['/settings/usage'] });

    fireEvent.click(screen.getByTestId('usage-tab-background'));

    await screen.findByTestId('stub-background-loops');
    expect(screen.getByTestId('usage-tab-background')).toHaveAttribute('aria-selected', 'true');
    expect(screen.queryByTestId('stub-cost-dashboard')).not.toBeInTheDocument();
  });

  test('clicking Costs from the Background tab restores the dashboard', async () => {
    renderWithProviders(<UsagePanel />, { initialEntries: ['/settings/usage#background'] });
    await screen.findByTestId('stub-background-loops');

    fireEvent.click(screen.getByTestId('usage-tab-costs'));

    await screen.findByTestId('stub-cost-dashboard');
    expect(screen.getByTestId('usage-tab-costs')).toHaveAttribute('aria-selected', 'true');
  });

  test('surfaces a snapshot load failure on the Background tab', async () => {
    mockLoad.mockRejectedValue(new Error('rpc down'));
    renderWithProviders(<UsagePanel />, { initialEntries: ['/settings/usage#background'] });

    await waitFor(() =>
      expect(screen.getByTestId('usage-background-tab')).toHaveTextContent(/rpc down/)
    );
    expect(screen.queryByTestId('stub-background-loops')).not.toBeInTheDocument();
  });
});
