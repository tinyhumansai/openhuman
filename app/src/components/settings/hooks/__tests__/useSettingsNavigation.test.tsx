import { screen } from '@testing-library/react';
import { describe, expect, test } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import { useSettingsNavigation } from '../useSettingsNavigation';

/** Renders breadcrumb labels so we can assert on the hook output. */
const BreadcrumbProbe = () => {
  const { breadcrumbs } = useSettingsNavigation();
  return <div data-testid="breadcrumbs">{breadcrumbs.map(b => b.label).join(' > ')}</div>;
};

/**
 * The two-pane settings restructure retired breadcrumb navigation — the sidebar
 * replaced the trail, so `breadcrumbs` is now always empty regardless of route.
 * The field is retained (always []) so the many panel call sites keep compiling.
 * Route resolution itself is covered in useSettingsNavigation.coverage.test.tsx.
 */
describe('useSettingsNavigation breadcrumbs (retired — always empty)', () => {
  const routes = [
    '/settings',
    '/settings/notifications',
    '/settings/tasks',
    '/settings/developer-options',
    '/settings/personality',
    '/settings/recovery-phrase',
    '/settings/wallet-balances',
    '/settings/notification-routing',
  ];

  test.each(routes)('breadcrumbs are empty for %s', route => {
    renderWithProviders(<BreadcrumbProbe />, { initialEntries: [route] });
    expect(screen.getByTestId('breadcrumbs')).toHaveTextContent('');
  });
});
