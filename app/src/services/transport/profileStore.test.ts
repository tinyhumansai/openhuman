/**
 * profileStore tests — desktop and iOS save/load/delete round-trip.
 *
 * Both paths currently use localStorage (iOS uses the same storage as desktop
 * since the WKWebView is app-sandboxed). The test ensures the public API
 * works correctly on both platform branches.
 */
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { clearTestPlatform, setTestPlatform } from '../../lib/platform';
import {
  type ConnectionProfile,
  deleteProfile,
  getProfile,
  listProfileIds,
  listProfiles,
  saveProfile,
} from './profileStore';

// -- helpers -----------------------------------------------------------------

function makeProfile(overrides: Partial<ConnectionProfile> = {}): ConnectionProfile {
  return {
    id: 'test-channel-id',
    label: 'Test Desktop',
    kind: 'tunnel',
    channelId: 'test-channel-id',
    pairingToken: 'test-pairing-token',
    corePubkey: 'test-core-pubkey',
    devicePrivkey: 'test-device-privkey',
    ...overrides,
  };
}

// -- setup -------------------------------------------------------------------

beforeEach(() => {
  // Clear localStorage between tests.
  localStorage.clear();
});

afterEach(() => {
  clearTestPlatform();
  localStorage.clear();
});

// -- desktop path ------------------------------------------------------------

describe('profileStore (desktop)', () => {
  beforeEach(() => {
    setTestPlatform('desktop');
  });

  it('save then get returns the same profile', () => {
    const profile = makeProfile();
    saveProfile(profile);
    const loaded = getProfile(profile.id);
    expect(loaded).not.toBeNull();
    expect(loaded?.id).toBe(profile.id);
    expect(loaded?.kind).toBe('tunnel');
    expect(loaded?.channelId).toBe(profile.channelId);
  });

  it('listProfileIds returns saved id', () => {
    const profile = makeProfile();
    saveProfile(profile);
    expect(listProfileIds()).toContain(profile.id);
  });

  it('listProfiles returns full profile objects', () => {
    const profile = makeProfile();
    saveProfile(profile);
    const profiles = listProfiles();
    expect(profiles).toHaveLength(1);
    expect(profiles[0].label).toBe('Test Desktop');
  });

  it('delete removes profile from store', () => {
    const profile = makeProfile();
    saveProfile(profile);
    deleteProfile(profile.id);
    expect(getProfile(profile.id)).toBeNull();
    expect(listProfileIds()).not.toContain(profile.id);
  });

  it('save multiple profiles', () => {
    saveProfile(makeProfile({ id: 'a', label: 'A' }));
    saveProfile(makeProfile({ id: 'b', label: 'B' }));
    expect(listProfileIds()).toHaveLength(2);
    expect(listProfiles().map(p => p.id)).toContain('a');
    expect(listProfiles().map(p => p.id)).toContain('b');
  });

  it('overwrite (same id) replaces label', () => {
    saveProfile(makeProfile({ id: 'x', label: 'Old' }));
    saveProfile(makeProfile({ id: 'x', label: 'New' }));
    expect(listProfileIds()).toHaveLength(1);
    expect(getProfile('x')?.label).toBe('New');
  });

  it('getProfile returns null for missing id', () => {
    expect(getProfile('does-not-exist')).toBeNull();
  });
});

// -- iOS path ----------------------------------------------------------------

describe('profileStore (iOS)', () => {
  beforeEach(() => {
    setTestPlatform('ios');
  });

  it('save then get round-trip works on iOS', () => {
    const profile = makeProfile({ id: 'ios-channel', label: 'iPhone 15' });
    saveProfile(profile);
    const loaded = getProfile('ios-channel');
    expect(loaded).not.toBeNull();
    expect(loaded?.label).toBe('iPhone 15');
    expect(loaded?.kind).toBe('tunnel');
  });

  it('listProfiles returns iOS profile', () => {
    const profile = makeProfile({ id: 'ios-chan', label: 'iPad' });
    saveProfile(profile);
    const all = listProfiles();
    expect(all).toHaveLength(1);
    expect(all[0].id).toBe('ios-chan');
  });

  it('delete removes profile on iOS', () => {
    const profile = makeProfile({ id: 'ios-del', label: 'Old Phone' });
    saveProfile(profile);
    deleteProfile('ios-del');
    expect(getProfile('ios-del')).toBeNull();
    expect(listProfiles()).toHaveLength(0);
  });

  it('devicePrivkey round-trips (stores and retrieves sensitive field)', () => {
    const profile = makeProfile({ id: 'ios-key', devicePrivkey: 'super-secret-private-key-value' });
    saveProfile(profile);
    const loaded = getProfile('ios-key');
    // Verify the field is present (it survives the JSON round-trip).
    expect(loaded?.devicePrivkey).toBe('super-secret-private-key-value');
    // SECURITY NOTE: in production this will be migrated to Keychain (Layer 7).
  });
});
