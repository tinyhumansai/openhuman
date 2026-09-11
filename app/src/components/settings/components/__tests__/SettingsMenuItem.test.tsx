import { fireEvent, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import SettingsMenuItem from '../SettingsMenuItem';

describe('<SettingsMenuItem />', () => {
  it('renders as a clickable button and fires onClick', () => {
    const onClick = vi.fn();
    renderWithProviders(
      <SettingsMenuItem
        icon={<svg aria-hidden="true" />}
        title="Log out"
        description="Sign out of this device"
        onClick={onClick}
        testId="settings-menu-logout"
      />
    );

    const button = screen.getByTestId('settings-menu-logout');
    expect(button.tagName).toBe('BUTTON');
    expect(screen.getByText('Log out')).toBeInTheDocument();
    expect(screen.getByText('Sign out of this device')).toBeInTheDocument();

    fireEvent.click(button);
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it('renders a static row when no onClick is provided', () => {
    renderWithProviders(
      <SettingsMenuItem
        icon={<svg aria-hidden="true" />}
        title="Version"
        testId="settings-menu-version"
      />
    );

    const row = screen.getByTestId('settings-menu-version');
    expect(row.tagName).toBe('DIV');
    expect(screen.getByText('Version')).toBeInTheDocument();
  });

  it('renders the right-side element when provided', () => {
    renderWithProviders(
      <SettingsMenuItem
        icon={<svg aria-hidden="true" />}
        title="Plan"
        rightElement={<span data-testid="right-badge">Pro</span>}
        testId="settings-menu-plan"
      />
    );

    expect(screen.getByTestId('right-badge')).toHaveTextContent('Pro');
  });
});
