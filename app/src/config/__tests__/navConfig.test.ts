import { describe, expect, it } from 'vitest';

import { AVATAR_MENU_ITEMS, NAV_TABS } from '../navConfig';

describe('NAV_TABS', () => {
  it('has exactly 5 entries', () => {
    expect(NAV_TABS).toHaveLength(5);
  });

  it('has the correct ids in order', () => {
    expect(NAV_TABS.map(t => t.id)).toEqual([
      'chat',
      'human',
      'brain',
      'agent-world',
      'connections',
    ]);
  });

  it('has the correct paths', () => {
    expect(NAV_TABS.map(t => t.path)).toEqual([
      '/chat',
      '/human',
      '/brain',
      '/agent-world',
      '/connections',
    ]);
  });

  it('has the correct labelKeys', () => {
    expect(NAV_TABS.map(t => t.labelKey)).toEqual([
      'nav.chat',
      'nav.human',
      'nav.brain',
      'nav.agentWorld',
      'nav.connections',
    ]);
  });

  it('has the correct walkthroughAttrs', () => {
    expect(NAV_TABS.map(t => t.walkthroughAttr)).toEqual([
      'tab-chat',
      'tab-human',
      'tab-brain',
      'tab-agent-world',
      'tab-connections',
    ]);
  });

  it('no longer contains home or settings tabs (moved to the sidebar header)', () => {
    expect(NAV_TABS.find(t => t.id === 'home')).toBeUndefined();
    expect(NAV_TABS.find(t => t.id === 'settings')).toBeUndefined();
  });

  it('no longer contains a feedback tab (moved to the sidebar footer row)', () => {
    expect(NAV_TABS.find(t => t.id === 'feedback')).toBeUndefined();
  });

  it('does not contain an activity tab', () => {
    expect(NAV_TABS.find(t => t.id === 'activity')).toBeUndefined();
  });

  it('does not contain a rewards tab', () => {
    expect(NAV_TABS.find(t => t.id === 'rewards')).toBeUndefined();
  });

  it('does not contain an intelligence or skills tab id', () => {
    expect(NAV_TABS.find(t => t.id === 'intelligence')).toBeUndefined();
    expect(NAV_TABS.find(t => t.id === 'skills')).toBeUndefined();
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

  it('billing, rewards, and invites are cloudOnly; account and wallet are not', () => {
    const cloudOnly = AVATAR_MENU_ITEMS.filter(i => i.cloudOnly).map(i => i.id);
    expect(cloudOnly).toEqual(['billing', 'rewards', 'invites']);
  });

  it('billing uses openUrl; all others use navigate', () => {
    const openUrlItems = AVATAR_MENU_ITEMS.filter(i => i.kind === 'openUrl').map(i => i.id);
    expect(openUrlItems).toEqual(['billing']);
  });
});
