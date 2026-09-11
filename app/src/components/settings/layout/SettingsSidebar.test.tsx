import { fireEvent, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import SettingsSidebar from './SettingsSidebar';

const navigateToSettings = vi.fn();

vi.mock('../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ currentRoute: 'privacy', navigateToSettings }),
}));

vi.mock('../settingsRouteRegistry', () => ({
  NAV_GROUP_LABEL_KEY: { general: 'nav.group.general' },
  entryRoute: (entry: { id: string }) => entry.id,
  resolveSidebarId: (routeId: string) => routeId,
  sidebarGroups: () => [
    {
      group: 'general',
      entries: [
        { id: 'privacy', titleKey: 'Privacy', route: 'privacy' },
        { id: 'account', titleKey: 'Account', route: 'account' },
      ],
    },
  ],
}));

vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

describe('<SettingsSidebar />', () => {
  it('renders grouped nav rows and marks the active one', () => {
    renderWithProviders(<SettingsSidebar />);

    expect(screen.getByTestId('settings-nav-privacy')).toHaveAttribute('aria-current', 'page');
    expect(screen.getByTestId('settings-nav-account')).not.toHaveAttribute('aria-current');
  });

  it('navigates when a nav row is clicked', () => {
    renderWithProviders(<SettingsSidebar />);

    fireEvent.click(screen.getByTestId('settings-nav-account'));
    expect(navigateToSettings).toHaveBeenCalledWith('account');
  });
});
