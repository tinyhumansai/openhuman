import debugFactory from 'debug';
import { useCallback } from 'react';
import { type Location, useLocation, useNavigate } from 'react-router-dom';

import { SETTINGS_FALLBACK_PATH } from './settingsOverlay';

const debug = debugFactory('settings:modal');

interface SettingsLocationState {
  backgroundLocation?: Location;
}

/**
 * Returns a callback that closes the Settings modal by navigating back to the
 * page it was opened over (`history.state.backgroundLocation`), falling back to
 * {@link SETTINGS_FALLBACK_PATH} for deep links / redirects that carry no
 * background. Shared by the X button, Esc, and backdrop click.
 */
export function useCloseSettings(): () => void {
  const navigate = useNavigate();
  const location = useLocation();

  return useCallback(() => {
    const background = (location.state as SettingsLocationState | null)?.backgroundLocation;
    debug('closeSettings: background=%o', background ?? SETTINGS_FALLBACK_PATH);
    // replace so pressing Back after closing doesn't reopen the Settings modal.
    navigate(background ?? SETTINGS_FALLBACK_PATH, { replace: true });
  }, [navigate, location.state]);
}
