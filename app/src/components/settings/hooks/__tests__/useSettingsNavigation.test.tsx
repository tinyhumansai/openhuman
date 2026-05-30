import { screen } from '@testing-library/react';
import { describe, expect, test } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import { useSettingsNavigation } from '../useSettingsNavigation';

/** Renders breadcrumb labels so we can assert on the hook output. */
const BreadcrumbProbe = () => {
  const { breadcrumbs } = useSettingsNavigation();
  return <div data-testid="breadcrumbs">{breadcrumbs.map(b => b.label).join(' > ')}</div>;
};

describe('useSettingsNavigation breadcrumbs', () => {
  test('notification-routing returns Settings > Developer Options', () => {
    renderWithProviders(<BreadcrumbProbe />, {
      initialEntries: ['/settings/notification-routing'],
    });
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('Settings > Developer Options');
  });

  test('notifications-hub returns Settings > Developer Options', () => {
    renderWithProviders(<BreadcrumbProbe />, { initialEntries: ['/settings/notifications-hub'] });
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('Settings > Developer Options');
  });

  test('notifications panel nests under Settings > Developer Options > Notifications', () => {
    renderWithProviders(<BreadcrumbProbe />, { initialEntries: ['/settings/notifications'] });
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent(
      'Settings > Developer Options > Notifications'
    );
  });

  test('developer-options returns Settings (section page)', () => {
    renderWithProviders(<BreadcrumbProbe />, { initialEntries: ['/settings/developer-options'] });
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('Settings');
  });

  test('persona returns Settings (top-level)', () => {
    renderWithProviders(<BreadcrumbProbe />, { initialEntries: ['/settings/persona'] });
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('Settings');
  });

  test('crypto returns Settings (section page)', () => {
    renderWithProviders(<BreadcrumbProbe />, { initialEntries: ['/settings/crypto'] });
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('Settings');
  });

  test('recovery-phrase returns Settings > Crypto', () => {
    renderWithProviders(<BreadcrumbProbe />, { initialEntries: ['/settings/recovery-phrase'] });
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('Settings > Crypto');
  });

  test('wallet-balances returns Settings > Crypto', () => {
    renderWithProviders(<BreadcrumbProbe />, { initialEntries: ['/settings/wallet-balances'] });
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('Settings > Crypto');
  });
});
