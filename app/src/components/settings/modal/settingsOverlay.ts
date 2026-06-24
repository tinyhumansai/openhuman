// [settings:modal] Pure routing helpers for the desktop Settings modal overlay.
//
// Desktop Settings is presented as a centered modal floating over the page the
// user was on. We use react-router's "background location" pattern: opening
// Settings stashes the current location in `history.state.backgroundLocation`,
// the shell renders that page underneath, and the `/settings/*` route renders
// inside the modal on top. The URL still becomes `/settings/account` etc. so
// deep links, settings search, and legacy redirects keep working.
//
// This module is pure (no JSX/React) so the branching logic is unit-testable in
// isolation without mounting the app or importing the panel barrel.
import debugFactory from 'debug';
import type { Location } from 'react-router-dom';

const debug = debugFactory('settings:modal');

/**
 * Page rendered behind the modal when Settings is reached via a deep link or a
 * redirect that carries no `backgroundLocation` (e.g. `/activity` →
 * `/settings/notifications`). Guarantees the overlay always has a sane backdrop.
 */
export const SETTINGS_FALLBACK_PATH = '/chat';

/** Shape of the nav state we thread through history to remember the backdrop. */
interface SettingsLocationState {
  backgroundLocation?: Location;
}

/** True when a pathname targets the settings route tree. */
export function isSettingsPath(pathname: string): boolean {
  return pathname === '/settings' || pathname.startsWith('/settings/');
}

/**
 * Resolve the location that should render *behind* the modal for a navigation.
 *
 * - Already on a settings path (switching panels inside the modal): preserve the
 *   existing `backgroundLocation` so the backdrop never changes.
 * - Anywhere else (opening Settings): the current location *is* the backdrop.
 *
 * Returns `undefined` only when we're on a settings path with no stored
 * background (deep link / redirect) — callers fall back to {@link
 * SETTINGS_FALLBACK_PATH}.
 */
export function resolveBackgroundLocation(location: Location): Location | undefined {
  if (isSettingsPath(location.pathname)) {
    return (location.state as SettingsLocationState | null)?.backgroundLocation;
  }
  return location;
}

/**
 * `navigate(...)` options that carry the resolved backdrop. Spread into any
 * navigation that opens or moves within Settings:
 *   navigate('/settings/billing', settingsNavState(location))
 */
export function settingsNavState(location: Location): { state: { backgroundLocation?: Location } } {
  return { state: { backgroundLocation: resolveBackgroundLocation(location) } };
}

/**
 * Drive the desktop shell: whether the Settings modal is open and which
 * location the page *behind* it should render.
 */
export function resolveSettingsOverlay(location: Location): {
  settingsOpen: boolean;
  baseLocation: Location | string;
} {
  const settingsOpen = isSettingsPath(location.pathname);
  const background = (location.state as SettingsLocationState | null)?.backgroundLocation;
  const baseLocation = background ?? (settingsOpen ? SETTINGS_FALLBACK_PATH : location);
  debug(
    'resolveSettingsOverlay: path=%s open=%s base=%o',
    location.pathname,
    settingsOpen,
    background ?? (settingsOpen ? SETTINGS_FALLBACK_PATH : location.pathname)
  );
  return { settingsOpen, baseLocation };
}
