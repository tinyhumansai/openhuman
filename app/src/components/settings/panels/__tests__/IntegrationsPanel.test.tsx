import { fireEvent, screen } from '@testing-library/react';
import { useLocation } from 'react-router-dom';
import { describe, expect, test, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import IntegrationsPanel from '../IntegrationsPanel';

// Surfaces the current router location so we can assert legacy-hash redirects.
const LocationProbe = () => {
  const location = useLocation();
  return <div data-testid="location-probe">{`${location.pathname}${location.search}`}</div>;
};

// The tab bodies have their own test suites — stub them so these tests stay
// focused on the hash <-> tab mapping that IntegrationsPanel owns.
vi.mock('../TaskSourcesPanel', () => ({
  default: ({ embedded }: { embedded?: boolean }) => (
    <div data-testid="stub-task-sources" data-embedded={String(embedded ?? false)} />
  ),
}));

vi.mock('../../../../pages/Webhooks', () => ({
  default: ({ embedded }: { embedded?: boolean }) => (
    <div data-testid="stub-webhooks" data-embedded={String(embedded ?? false)} />
  ),
}));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: vi.fn(),
    navigateToSettings: vi.fn(),
    breadcrumbs: [],
  }),
}));

describe('IntegrationsPanel', () => {
  test('default hash renders the Task sources tab embedded', () => {
    renderWithProviders(<IntegrationsPanel />, { initialEntries: ['/settings/integrations'] });

    expect(screen.getByTestId('integrations-tab-task-sources')).toHaveAttribute(
      'aria-selected',
      'true'
    );
    expect(screen.getByTestId('stub-task-sources')).toHaveAttribute('data-embedded', 'true');
    expect(screen.queryByTestId('stub-webhooks')).not.toBeInTheDocument();
  });

  test('#webhooks hash selects the Webhooks tab embedded', () => {
    renderWithProviders(<IntegrationsPanel />, {
      initialEntries: ['/settings/integrations#webhooks'],
    });

    expect(screen.getByTestId('integrations-tab-webhooks')).toHaveAttribute(
      'aria-selected',
      'true'
    );
    expect(screen.getByTestId('stub-webhooks')).toHaveAttribute('data-embedded', 'true');
  });

  test('legacy #composio hash redirects to Connections → API keys', () => {
    renderWithProviders(
      <>
        <IntegrationsPanel />
        <LocationProbe />
      </>,
      { initialEntries: ['/settings/integrations#composio'] }
    );

    expect(screen.getByTestId('location-probe')).toHaveTextContent('/connections?tab=composio-key');
  });

  test('clicking tabs switches the view in place', async () => {
    renderWithProviders(<IntegrationsPanel />, { initialEntries: ['/settings/integrations'] });

    fireEvent.click(screen.getByTestId('integrations-tab-webhooks'));
    await screen.findByTestId('stub-webhooks');

    fireEvent.click(screen.getByTestId('integrations-tab-task-sources'));
    await screen.findByTestId('stub-task-sources');
  });
});
