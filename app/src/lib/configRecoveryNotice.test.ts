import { beforeEach, describe, expect, it } from 'vitest';

import { store } from '../store';
import { __resetForTests, maybeSurfaceConfigRecovery } from './configRecoveryNotice';

// Fallback translator matching `useT()`'s `t(key, fallback?)` contract: return
// the English fallback so assertions read the default copy, and echo the key
// when no fallback is given so key wiring is observable.
const t = (key: string, fallback?: string): string => fallback ?? key;

describe('maybeSurfaceConfigRecovery', () => {
  beforeEach(() => {
    __resetForTests();
    store.dispatch({ type: 'notifications/clearAll' });
  });

  it('surfaces a single system notice when config was recovered', () => {
    maybeSurfaceConfigRecovery(true, t);
    const items = store.getState().notifications.items;
    expect(items).toHaveLength(1);
    expect(items[0].id).toBe('config-recovered');
    expect(items[0].category).toBe('system');
    expect(items[0].title).toBe('Settings file recovered');
    // Copy must be accurate for both recovery outcomes (backup restore OR
    // defaults reset) — it must not hard-claim "reset to defaults" (#5167).
    expect(items[0].body).toContain('restored from a backup or reset to');
    expect(items[0].read).toBe(false);
    expect(items[0].deepLink).toBe('/settings');
  });

  it('localizes via the translator keys', () => {
    // A key-echoing translator proves the notice pulls copy from i18n keys
    // rather than hardcoded strings.
    maybeSurfaceConfigRecovery(true, key => key);
    const item = store.getState().notifications.items[0];
    expect(item.title).toBe('notifications.configRecovered.title');
    expect(item.body).toBe('notifications.configRecovered.body');
  });

  it('is a one-shot — repeated polls do not re-dispatch', () => {
    maybeSurfaceConfigRecovery(true, t);
    maybeSurfaceConfigRecovery(true, t);
    maybeSurfaceConfigRecovery(true, t);
    expect(store.getState().notifications.items).toHaveLength(1);
  });

  it('does nothing when configRecovered is false or absent', () => {
    maybeSurfaceConfigRecovery(false, t);
    maybeSurfaceConfigRecovery(undefined, t);
    expect(store.getState().notifications.items).toHaveLength(0);
  });
});
