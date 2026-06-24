import { fireEvent, render, screen } from '@testing-library/react';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { describe, expect, it } from 'vitest';

import { useCloseSettings } from './useCloseSettings';

/** Probe that closes on click and echoes the current path. */
function Probe() {
  const close = useCloseSettings();
  const location = useLocation();
  return (
    <button type="button" data-testid="probe" data-path={location.pathname} onClick={close}>
      close
    </button>
  );
}

const currentPath = () => screen.getByTestId('probe').getAttribute('data-path');

describe('useCloseSettings', () => {
  it('navigates to the stored backgroundLocation when present', () => {
    render(
      <MemoryRouter
        initialEntries={[
          { pathname: '/settings/account', state: { backgroundLocation: { pathname: '/brain' } } },
        ]}>
        <Probe />
      </MemoryRouter>
    );
    expect(currentPath()).toBe('/settings/account');
    fireEvent.click(screen.getByTestId('probe'));
    expect(currentPath()).toBe('/brain');
  });

  it('falls back to /chat when there is no backgroundLocation', () => {
    render(
      <MemoryRouter initialEntries={[{ pathname: '/settings/voice' }]}>
        <Probe />
      </MemoryRouter>
    );
    fireEvent.click(screen.getByTestId('probe'));
    expect(currentPath()).toBe('/chat');
  });
});
