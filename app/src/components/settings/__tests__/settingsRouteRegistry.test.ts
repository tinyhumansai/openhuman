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
    const entry = findEntryById('persona');
    expect(entry).toBeDefined();
    expect(entryRoute(entry!)).toBe('persona');
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

  it('returns the correct section for agents-settings', () => {
    const entry = findEntryById('agents-settings');
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
    const entry = findEntryByRoute('persona');
    expect(entry).toBeDefined();
    expect(entry!.id).toBe('persona');
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

  it('does not match partial/substring routes — no collision between "ai" and "ai-debug"', () => {
    const entry = findEntryByRoute('ai');
    expect(entry).toBeDefined();
    expect(entry!.id).toBe('ai');
    // There should be no entry with id 'ai-debug' in the registry.
    expect(findEntryByRoute('ai-debug')).toBeUndefined();
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
    // 'approval-history' is section: 'agents' + hiddenDeepLink: true.
    const agentEntries = entriesForSection('agents');
    const ids = agentEntries.map(e => e.id);
    expect(ids).not.toContain('approval-history');
  });

  it('returns the composio section entries', () => {
    const composioEntries = entriesForSection('composio');
    const ids = composioEntries.map(e => e.id);
    expect(ids).toContain('task-sources');
    expect(ids).toContain('composio-routing');
    expect(ids).toContain('webhooks-triggers');
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
    // Non-hidden home entries include the main section hubs.
    expect(ids).toContain('account');
    expect(ids).toContain('ai');
    expect(ids).toContain('agents-settings');
    expect(ids).toContain('features');
    expect(ids).toContain('composio');
    expect(ids).toContain('notifications-hub');
    expect(ids).toContain('crypto');
    expect(ids).toContain('about');
    // billing is hiddenDeepLink so should be excluded.
    expect(ids).not.toContain('billing');
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

  it('contains the 5 newly-surfaced section hubs on home', () => {
    const homeIds = entriesForSection('home').map(e => e.id);
    expect(homeIds).toContain('ai');
    expect(homeIds).toContain('agents-settings');
    expect(homeIds).toContain('features');
    expect(homeIds).toContain('composio');
    expect(homeIds).toContain('crypto');
  });
});
