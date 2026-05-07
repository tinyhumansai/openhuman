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

  test('notifications returns Settings (top-level)', () => {
    renderWithProviders(<BreadcrumbProbe />, { initialEntries: ['/settings/notifications'] });
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('Settings');
  });

  test('developer-options returns Settings (section page)', () => {
    renderWithProviders(<BreadcrumbProbe />, { initialEntries: ['/settings/developer-options'] });
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('Settings');
  });
});
