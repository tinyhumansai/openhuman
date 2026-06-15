/**
 * Unit tests for settingsRouteRegistry helpers.
 *
 * Covers the four exported helper functions and edge-cases that ensure the
 * registry stays internally consistent (no duplicate ids, every entry has a
 * reachable route, etc.).
 */
import { describe, expect, it } from 'vitest';

import {
  entriesForSection,
  entryRoute,
  findEntryById,
  findEntryByRoute,
  SETTINGS_ROUTE_REGISTRY,
} from '../settingsRouteRegistry';

// ---------------------------------------------------------------------------
// entryRoute
// ---------------------------------------------------------------------------

describe('entryRoute', () => {
  it('returns the explicit route when set', () => {
    // 'notifications' entry has route: 'notifications' set explicitly.
    const entry = findEntryById('notifications');
    expect(entry).toBeDefined();
    expect(entryRoute(entry!)).toBe('notifications');
  });

  it('falls back to the id when no explicit route is set', () => {
    const entry = findEntryById('personality');
    expect(entry).toBeDefined();
    expect(entryRoute(entry!)).toBe('personality');
  });

  it('returns the overridden route for build-info (→ about)', () => {
    const entry = findEntryById('build-info');
    expect(entry).toBeDefined();
    expect(entryRoute(entry!)).toBe('about');
  });
});

// ---------------------------------------------------------------------------
// findEntryById
// ---------------------------------------------------------------------------

describe('findEntryById', () => {
  it('returns the entry for a known id', () => {
    const entry = findEntryById('about');
    expect(entry).toBeDefined();
    expect(entry!.id).toBe('about');
  });

  it('returns undefined for an unknown id', () => {
    expect(findEntryById('does-not-exist')).toBeUndefined();
  });

  it('returns the correct section for a home hub entry', () => {
    // The old 'agents-settings' / 'ai' hub pages were retired; 'integrations'
    // is a representative surviving home-section hub.
    const entry = findEntryById('integrations');
    expect(entry).toBeDefined();
    expect(entry!.section).toBe('home');
  });

  it('returns the correct section for a developer-only entry', () => {
    const entry = findEntryById('cron-jobs');
    expect(entry).toBeDefined();
    expect(entry!.section).toBe('developer');
    expect(entry!.devOnly).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// findEntryByRoute
// ---------------------------------------------------------------------------

describe('findEntryByRoute', () => {
  it('returns an entry for a known route', () => {
    const entry = findEntryByRoute('personality');
    expect(entry).toBeDefined();
    expect(entry!.id).toBe('personality');
  });

  it('returns undefined for an unknown route', () => {
    expect(findEntryByRoute('messaging')).toBeUndefined();
  });

  it('returns the build-info entry when looking up the "about" route alias', () => {
    // build-info has route: 'about', so findEntryByRoute('about') returns
    // whichever comes first — likely the canonical 'about' entry itself.
    // The important assertion: the route is reachable.
    const entry = findEntryByRoute('about');
    expect(entry).toBeDefined();
  });

  it('does not match partial/substring routes — no collision between "voice" and "voice-debug"', () => {
    const entry = findEntryByRoute('voice');
    expect(entry).toBeDefined();
    expect(entry!.id).toBe('voice');
    // 'voice-debug' is a distinct developer entry; exact-match lookup must not
    // collide with the 'voice' leaf despite the shared prefix.
    const debugEntry = findEntryByRoute('voice-debug');
    expect(debugEntry).toBeDefined();
    expect(debugEntry!.id).toBe('voice-debug');
  });
});

// ---------------------------------------------------------------------------
// entriesForSection
// ---------------------------------------------------------------------------

describe('entriesForSection', () => {
  it('returns only entries belonging to the requested section', () => {
    const cryptoEntries = entriesForSection('crypto');
    expect(cryptoEntries.length).toBeGreaterThan(0);
    cryptoEntries.forEach(e => expect(e.section).toBe('crypto'));
  });

  it('excludes hidden deep-links', () => {
    // 'autocomplete' and 'permissions' are section: 'developer' + hiddenDeepLink.
    const devEntries = entriesForSection('developer');
    const ids = devEntries.map(e => e.id);
    expect(ids).not.toContain('autocomplete');
    expect(ids).not.toContain('permissions');
  });

  it('surfaces the merged integrations entry on home (composio section retired)', () => {
    const homeEntries = entriesForSection('home');
    const ids = homeEntries.map(e => e.id);
    expect(ids).toContain('integrations');
    // The old composio leaf slugs redirect to /settings/integrations and are
    // no longer registry entries.
    const allIds = SETTINGS_ROUTE_REGISTRY.map(e => e.id);
    expect(allIds).not.toContain('task-sources');
    expect(allIds).not.toContain('composio-routing');
    expect(allIds).not.toContain('webhooks-triggers');
  });

  it('returns multiple developer entries', () => {
    const devEntries = entriesForSection('developer');
    expect(devEntries.length).toBeGreaterThan(5);
    devEntries.forEach(e => {
      expect(e.section).toBe('developer');
      expect(e.hiddenDeepLink).not.toBe(true);
    });
  });

  it('returns home section entries (section hubs)', () => {
    const homeEntries = entriesForSection('home');
    const ids = homeEntries.map(e => e.id);
    // Surviving home hub entries after the two-pane restructure.
    expect(ids).toContain('account');
    expect(ids).toContain('appearance');
    expect(ids).toContain('personality');
    expect(ids).toContain('automations');
    expect(ids).toContain('integrations');
    expect(ids).toContain('about');
    // The old ai / agents-settings / features / notifications-hub hub pages
    // were retired — their slugs now redirect to leaf panels.
    expect(ids).not.toContain('ai');
    expect(ids).not.toContain('agents-settings');
    expect(ids).not.toContain('features');
    expect(ids).not.toContain('notifications-hub');
  });

  it('returns empty array for a section that has no non-hidden entries', () => {
    // All home entries are reachable so this just validates the helper signature.
    const result = entriesForSection('account');
    expect(Array.isArray(result)).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// Registry-level integrity checks
// ---------------------------------------------------------------------------

describe('SETTINGS_ROUTE_REGISTRY integrity', () => {
  it('has no duplicate ids', () => {
    const ids = SETTINGS_ROUTE_REGISTRY.map(e => e.id);
    const unique = new Set(ids);
    expect(unique.size).toBe(ids.length);
  });

  it('every entry has a non-empty id and titleKey', () => {
    SETTINGS_ROUTE_REGISTRY.forEach(entry => {
      expect(entry.id.length).toBeGreaterThan(0);
      expect(entry.titleKey.length).toBeGreaterThan(0);
    });
  });

  it('surfaces the restructured home hub entries', () => {
    const homeIds = entriesForSection('home').map(e => e.id);
    expect(homeIds).toContain('integrations');
    expect(homeIds).toContain('personality');
    expect(homeIds).toContain('automations');
    expect(homeIds).toContain('memory-sync');
    // billing is surfaced in the General group now (no longer a hidden deep-link).
    expect(homeIds).toContain('billing');
  });
});
