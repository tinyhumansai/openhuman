/**
 * Tests for navConfig — verifies the shape, count, and key values of NAV_TABS
 * and AVATAR_MENU_ITEMS so regressions are caught early.
 *
 * Human tab restored as a first-class entry (after the IA Phase 6 merge into
 * Assistant), so the regular row is back to 6 tabs. The Assistant (id 'chat',
 * labelKey 'nav.assistant', walkthroughAttr 'tab-chat') is no longer in the
 * row — it's the raised center FAB (`CENTER_TAB`); the Brain sits in the row.
 */
import { describe, expect, it } from 'vitest';

import { AVATAR_MENU_ITEMS, CENTER_TAB, NAV_TABS } from '../navConfig';

describe('NAV_TABS', () => {
  it('has exactly 6 entries (Human restored as a first-class tab)', () => {
    expect(NAV_TABS).toHaveLength(6);
  });

  it('has the correct ids in order (Brain in the row, Assistant is the center FAB)', () => {
    expect(NAV_TABS.map(t => t.id)).toEqual([
      'home',
      'human',
      'brain',
      'connections',
      'activity',
      'settings',
    ]);
  });

  it('has the correct paths', () => {
    expect(NAV_TABS.map(t => t.path)).toEqual([
      '/home',
      '/human',
      '/brain',
      '/connections',
      '/activity',
      '/settings',
    ]);
  });

  it('has the correct labelKeys', () => {
    expect(NAV_TABS.map(t => t.labelKey)).toEqual([
      'nav.home',
      'nav.human',
      'nav.brain',
      'nav.connections',
      'nav.activity',
      'nav.settings',
    ]);
  });

  it('has the correct walkthroughAttrs', () => {
    expect(NAV_TABS.map(t => t.walkthroughAttr)).toEqual([
      'tab-home',
      'tab-human',
      'tab-brain',
      'tab-connections',
      'tab-activity',
      'tab-settings',
    ]);
  });

  it('contains a Human tab pointing at /human', () => {
    const humanTab = NAV_TABS.find(t => t.id === 'human');
    expect(humanTab).toBeDefined();
    expect(humanTab?.path).toBe('/human');
    expect(humanTab?.labelKey).toBe('nav.human');
    expect(humanTab?.walkthroughAttr).toBe('tab-human');
  });

  it('does not contain a rewards tab', () => {
    expect(NAV_TABS.find(t => t.id === 'rewards')).toBeUndefined();
  });

  it('does not contain an intelligence or skills tab id', () => {
    expect(NAV_TABS.find(t => t.id === 'intelligence')).toBeUndefined();
    expect(NAV_TABS.find(t => t.id === 'skills')).toBeUndefined();
  });

  it('does not contain the Assistant/chat tab (rendered specially as the center FAB)', () => {
    expect(NAV_TABS.find(t => t.id === 'chat')).toBeUndefined();
  });

  it('Brain tab sits in the regular row with nav.brain label and tab-brain walkthrough attr', () => {
    const brainTab = NAV_TABS.find(t => t.id === 'brain');
    expect(brainTab).toBeDefined();
    expect(brainTab?.labelKey).toBe('nav.brain');
    expect(brainTab?.walkthroughAttr).toBe('tab-brain');
    expect(brainTab?.path).toBe('/brain');
  });
});

describe('CENTER_TAB', () => {
  it('is the Assistant — expected shape (id, path, labelKey, walkthroughAttr)', () => {
    expect(CENTER_TAB).toEqual({
      id: 'chat',
      labelKey: 'nav.assistant',
      path: '/chat',
      walkthroughAttr: 'tab-chat',
    });
  });
});

describe('AVATAR_MENU_ITEMS', () => {
  it('has exactly 5 entries', () => {
    expect(AVATAR_MENU_ITEMS).toHaveLength(5);
  });

  it('has the correct ids in order', () => {
    expect(AVATAR_MENU_ITEMS.map(i => i.id)).toEqual([
      'account',
      'billing',
      'rewards',
      'invites',
      'wallet',
    ]);
  });

  it('has the correct labelKeys', () => {
    expect(AVATAR_MENU_ITEMS.map(i => i.labelKey)).toEqual([
      'nav.avatarMenu.account',
      'nav.avatarMenu.billing',
      'nav.avatarMenu.rewards',
      'nav.avatarMenu.invites',
      'nav.avatarMenu.wallet',
    ]);
  });

  it('billing, rewards, and invites are cloudOnly; account and wallet are not', () => {
    const cloudOnly = AVATAR_MENU_ITEMS.filter(i => i.cloudOnly).map(i => i.id);
    expect(cloudOnly).toEqual(['billing', 'rewards', 'invites']);

    const notCloudOnly = AVATAR_MENU_ITEMS.filter(i => !i.cloudOnly).map(i => i.id);
    expect(notCloudOnly).toEqual(['account', 'wallet']);
  });

  it('billing uses openUrl; all others use navigate', () => {
    const openUrlItems = AVATAR_MENU_ITEMS.filter(i => i.kind === 'openUrl').map(i => i.id);
    expect(openUrlItems).toEqual(['billing']);

    const navigateItems = AVATAR_MENU_ITEMS.filter(i => i.kind === 'navigate').map(i => i.id);
    expect(navigateItems).toEqual(['account', 'rewards', 'invites', 'wallet']);
  });

  it('each item has a non-empty target', () => {
    for (const item of AVATAR_MENU_ITEMS) {
      expect(item.target.length).toBeGreaterThan(0);
    }
  });
});
