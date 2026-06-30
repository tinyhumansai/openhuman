import type { Location } from 'react-router-dom';
import { describe, expect, it } from 'vitest';

import {
  isSettingsPath,
  resolveBackgroundLocation,
  resolveSettingsOverlay,
  SETTINGS_FALLBACK_PATH,
  settingsNavState,
} from './settingsOverlay';

/** Build a minimal Location for tests (cast avoids coupling to router internals). */
const loc = (pathname: string, state: unknown = null): Location =>
  ({ pathname, search: '', hash: '', state, key: 'test' }) as unknown as Location;

describe('isSettingsPath', () => {
  it('matches the settings index and sub-paths', () => {
    expect(isSettingsPath('/settings')).toBe(true);
    expect(isSettingsPath('/settings/account')).toBe(true);
    expect(isSettingsPath('/settings/team/manage/1')).toBe(true);
  });

  it('does not match other paths', () => {
    expect(isSettingsPath('/chat')).toBe(false);
    expect(isSettingsPath('/settings-foo')).toBe(false);
    expect(isSettingsPath('/')).toBe(false);
  });
});

describe('resolveBackgroundLocation', () => {
  it('captures the current location when opening from a non-settings page', () => {
    const here = loc('/brain');
    expect(resolveBackgroundLocation(here)).toBe(here);
  });

  it('preserves the stored background when already on a settings path', () => {
    const background = loc('/brain');
    expect(
      resolveBackgroundLocation(loc('/settings/billing', { backgroundLocation: background }))
    ).toBe(background);
  });

  it('returns undefined on a settings path with no stored background (deep link)', () => {
    expect(resolveBackgroundLocation(loc('/settings/voice'))).toBeUndefined();
  });
});

describe('settingsNavState', () => {
  it('wraps the resolved background in navigate options', () => {
    const here = loc('/rewards');
    expect(settingsNavState(here)).toEqual({ state: { backgroundLocation: here } });
  });

  it('preserves background across in-modal navigation', () => {
    const background = loc('/human');
    expect(settingsNavState(loc('/settings/account', { backgroundLocation: background }))).toEqual({
      state: { backgroundLocation: background },
    });
  });
});

describe('resolveSettingsOverlay', () => {
  it('reports closed and passes the current location through off settings', () => {
    const here = loc('/chat');
    expect(resolveSettingsOverlay(here)).toEqual({ settingsOpen: false, baseLocation: here });
  });

  it('reports open and renders the stored background behind', () => {
    const background = loc('/brain');
    expect(
      resolveSettingsOverlay(loc('/settings/account', { backgroundLocation: background }))
    ).toEqual({ settingsOpen: true, baseLocation: background });
  });

  it('falls back to /chat behind a deep-linked settings path', () => {
    expect(resolveSettingsOverlay(loc('/settings/voice'))).toEqual({
      settingsOpen: true,
      baseLocation: SETTINGS_FALLBACK_PATH,
    });
  });
});
