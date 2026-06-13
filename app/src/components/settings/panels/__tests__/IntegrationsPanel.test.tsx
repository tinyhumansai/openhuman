import { fireEvent, screen } from '@testing-library/react';
import { describe, expect, test, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import IntegrationsPanel from '../IntegrationsPanel';

// The tab bodies have their own test suites — stub them so these tests stay
// focused on the hash <-> tab mapping that IntegrationsPanel owns.
vi.mock('../TaskSourcesPanel', () => ({
  default: ({ embedded }: { embedded?: boolean }) => (
    <div data-testid="stub-task-sources" data-embedded={String(embedded ?? false)} />
  ),
}));

vi.mock('../ComposioPanel', () => ({
  default: ({ embedded }: { embedded?: boolean }) => (
    <div data-testid="stub-composio" data-embedded={String(embedded ?? false)} />
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
    expect(screen.queryByTestId('stub-composio')).not.toBeInTheDocument();
    expect(screen.queryByTestId('stub-webhooks')).not.toBeInTheDocument();
  });

  test('#composio hash selects the Composio tab embedded', () => {
    renderWithProviders(<IntegrationsPanel />, {
      initialEntries: ['/settings/integrations#composio'],
    });

    expect(screen.getByTestId('integrations-tab-composio')).toHaveAttribute(
      'aria-selected',
      'true'
    );
    expect(screen.getByTestId('stub-composio')).toHaveAttribute('data-embedded', 'true');
    expect(screen.queryByTestId('stub-task-sources')).not.toBeInTheDocument();
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

  test('clicking tabs switches the view in place', async () => {
    renderWithProviders(<IntegrationsPanel />, { initialEntries: ['/settings/integrations'] });

    fireEvent.click(screen.getByTestId('integrations-tab-composio'));
    await screen.findByTestId('stub-composio');

    fireEvent.click(screen.getByTestId('integrations-tab-webhooks'));
    await screen.findByTestId('stub-webhooks');

    fireEvent.click(screen.getByTestId('integrations-tab-task-sources'));
    await screen.findByTestId('stub-task-sources');
  });
});
