/**
 * Unit tests for the pure `searchSettings` matcher that powers the Settings
 * global search bar. Uses a small synthetic registry + fake translator so the
 * algorithm is tested in isolation from the real (churning) entry list.
 */
import { describe, expect, test } from 'vitest';

import type { SettingsSearchEntry } from './settingsSearchRegistry';
import { searchSettings } from './useSettingsSearch';

const LABELS: Record<string, string> = {
  'voice.t': 'Voice',
  'voice.d': 'Text to speech and dictation',
  'vd.t': 'Voice Debug',
  'priv.t': 'Privacy',
  'priv.d': 'Telemetry and tracking controls',
  'dataPriv.t': 'Data Privacy',
  'cafe.t': 'Café',
  'sec.ai': 'AI',
  'sec.dev': 'Developer',
  'sec.acct': 'Account',
};

const t = (key: string): string => LABELS[key] ?? key;

const ENTRIES: SettingsSearchEntry[] = [
  {
    id: 'voice',
    titleKey: 'voice.t',
    descriptionKey: 'voice.d',
    route: 'voice',
    sectionKey: 'sec.ai',
    keywords: ['speech', 'tts'],
  },
  {
    id: 'voice-debug',
    titleKey: 'vd.t',
    route: 'voice-debug',
    sectionKey: 'sec.dev',
    devOnly: true,
  },
  {
    id: 'privacy',
    titleKey: 'priv.t',
    descriptionKey: 'priv.d',
    route: 'privacy',
    sectionKey: 'sec.acct',
    keywords: ['telemetry'],
  },
  { id: 'data-privacy', titleKey: 'dataPriv.t', route: 'data-privacy', sectionKey: 'sec.acct' },
  { id: 'cafe', titleKey: 'cafe.t', route: 'cafe', sectionKey: 'sec.acct' },
];

const ids = (query: string, includeDevOnly = false) =>
  searchSettings(ENTRIES, query, t, includeDevOnly).map(r => r.entry.id);

describe('searchSettings', () => {
  test('empty / whitespace query returns no results', () => {
    expect(searchSettings(ENTRIES, '', t, true)).toEqual([]);
    expect(searchSettings(ENTRIES, '   ', t, true)).toEqual([]);
  });

  test('matches by title', () => {
    expect(ids('voice', true)).toContain('voice');
  });

  test('hides devOnly entries unless developer mode is enabled', () => {
    expect(ids('voice', false)).toEqual(['voice']);
    expect(ids('voice', true)).toEqual(['voice', 'voice-debug']);
  });

  test('matches by keyword synonym not present in the title', () => {
    expect(ids('tts')).toEqual(['voice']);
  });

  test('matches by description-only text', () => {
    // "controls" appears only in the privacy description, not its title/keywords.
    expect(ids('controls')).toEqual(['privacy']);
  });

  test('AND semantics — every token must match', () => {
    // "voice" alone hits both, but only Voice Debug contains "debug".
    expect(ids('voice debug', true)).toEqual(['voice-debug']);
  });

  test('is diacritic-insensitive', () => {
    expect(ids('cafe')).toEqual(['cafe']);
    expect(ids('café')).toEqual(['cafe']);
  });

  test('ranks a title-prefix match above a title-substring match', () => {
    // "Privacy" (prefix) should outrank "Data Privacy" (substring).
    expect(ids('priv')).toEqual(['privacy', 'data-privacy']);
  });
});
