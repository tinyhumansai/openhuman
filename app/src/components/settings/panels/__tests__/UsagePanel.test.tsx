import { fireEvent, screen } from '@testing-library/react';
import { useLocation } from 'react-router-dom';
import { describe, expect, test, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import UsagePanel from '../UsagePanel';

// The tab bodies are heavy (chart pipeline, multi-RPC fetches) — stub them so
// these tests stay focused on the hash <-> tab mapping that UsagePanel owns.
vi.mock('../../../dashboard/CostDashboardPanel', () => ({
  default: ({ embedded }: { embedded?: boolean }) => (
    <div data-testid="stub-cost-dashboard" data-embedded={String(embedded ?? false)} />
  ),
}));

// UsagePanel imports the loop-map component directly from its extracted
// module (`./ai/BackgroundLoopControls`), not through the `AIPanel` re-export
// — stub that module so this suite stays focused on the hash <-> tab mapping
// UsagePanel owns, rather than mounting the real component (and its
// RPC-backed data fetches).
vi.mock('../ai/BackgroundLoopControls', () => {
  const Stub = ({ view, hideHeader }: { view?: string; hideHeader?: boolean }) => (
    <div
      data-testid="stub-background-loops"
      data-view={view}
      data-hide-header={String(hideHeader ?? false)}
    />
  );
  return { BackgroundLoopControls: Stub, default: Stub };
});

vi.mock('../TokenUsagePanel', () => ({
  default: ({ embedded }: { embedded?: boolean }) => (
    <div data-testid="stub-token-usage" data-embedded={String(embedded ?? false)} />
  ),
}));

vi.mock('../../../dashboard/UsageLogPanel', () => ({
  default: () => <div data-testid="stub-usage-log" />,
}));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: vi.fn(),
    navigateToSettings: vi.fn(),
    breadcrumbs: [],
  }),
}));

const LocationProbe = () => {
  const location = useLocation();
  return <output data-testid="location-probe">{`${location.search}${location.hash}`}</output>;
};

describe('UsagePanel', () => {
  test('default hash renders the Costs tab with the embedded cost dashboard', () => {
    renderWithProviders(<UsagePanel />, { initialEntries: ['/settings/usage'] });

    expect(screen.getByTestId('usage-tab-costs')).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByTestId('usage-tab-background')).toHaveAttribute('aria-selected', 'false');
    expect(screen.getByTestId('stub-cost-dashboard')).toHaveAttribute('data-embedded', 'true');
  });

  test('#tokens hash selects the Token savings tab with the embedded TokenJuice panel', () => {
    renderWithProviders(<UsagePanel />, { initialEntries: ['/settings/usage#tokens'] });

    expect(screen.getByTestId('usage-tab-tokens')).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByTestId('stub-token-usage')).toHaveAttribute('data-embedded', 'true');
    expect(screen.queryByTestId('stub-cost-dashboard')).not.toBeInTheDocument();
  });

  test('#log hash selects the usage log without mounting the cost dashboard', () => {
    renderWithProviders(<UsagePanel />, { initialEntries: ['/settings/usage#log'] });

    expect(screen.getByTestId('usage-tab-log')).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByTestId('stub-usage-log')).toBeInTheDocument();
    expect(screen.queryByTestId('stub-cost-dashboard')).not.toBeInTheDocument();
  });

  test('#background hash selects the Background tab and renders the loop controls', async () => {
    renderWithProviders(<UsagePanel />, { initialEntries: ['/settings/usage#background'] });

    expect(screen.getByTestId('usage-tab-background')).toHaveAttribute('aria-selected', 'true');
    expect(screen.queryByTestId('stub-cost-dashboard')).not.toBeInTheDocument();

    const controls = await screen.findByTestId('stub-background-loops');
    expect(controls).toHaveAttribute('data-view', 'all');
    expect(controls).toHaveAttribute('data-hide-header', 'true');
  });

  test('clicking the Background tab switches the view in place', async () => {
    renderWithProviders(<UsagePanel />, { initialEntries: ['/settings/usage'] });

    fireEvent.click(screen.getByTestId('usage-tab-background'));

    await screen.findByTestId('stub-background-loops');
    expect(screen.getByTestId('usage-tab-background')).toHaveAttribute('aria-selected', 'true');
    expect(screen.queryByTestId('stub-cost-dashboard')).not.toBeInTheDocument();
  });

  test('preserves the Connections tab query when switching usage subtabs', async () => {
    renderWithProviders(
      <>
        <UsagePanel />
        <LocationProbe />
      </>,
      { initialEntries: ['/connections?tab=usage'] }
    );

    fireEvent.click(screen.getByTestId('usage-tab-background'));

    await screen.findByTestId('stub-background-loops');
    expect(screen.getByTestId('usage-tab-background')).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByTestId('location-probe')).toHaveTextContent('?tab=usage#background');
  });

  test('clicking Costs from the Background tab restores the dashboard', async () => {
    renderWithProviders(<UsagePanel />, { initialEntries: ['/settings/usage#background'] });
    await screen.findByTestId('stub-background-loops');

    fireEvent.click(screen.getByTestId('usage-tab-costs'));

    await screen.findByTestId('stub-cost-dashboard');
    expect(screen.getByTestId('usage-tab-costs')).toHaveAttribute('aria-selected', 'true');
  });
});
