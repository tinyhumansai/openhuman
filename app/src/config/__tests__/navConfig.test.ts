import { describe, expect, it } from 'vitest';

import { AVATAR_MENU_ITEMS, NAV_TABS } from '../navConfig';

describe('NAV_TABS', () => {
  it('has exactly 3 entries', () => {
    expect(NAV_TABS).toHaveLength(3);
  });

  it('has the correct ids in order', () => {
    expect(NAV_TABS.map(t => t.id)).toEqual(['chat', 'flows', 'connections']);
  });

  it('has the correct paths', () => {
    expect(NAV_TABS.map(t => t.path)).toEqual(['/chat', '/flows', '/connections']);
  });

  it('has the correct labelKeys', () => {
    expect(NAV_TABS.map(t => t.labelKey)).toEqual(['nav.chat', 'nav.flows', 'nav.connections']);
  });

  it('has the correct walkthroughAttrs', () => {
    expect(NAV_TABS.map(t => t.walkthroughAttr)).toEqual([
      'tab-chat',
      'tab-flows',
      'tab-connections',
    ]);
  });

  it('gates nothing on a cloud session now that Rewards is gone', () => {
    expect(NAV_TABS.filter(t => t.cloudOnly).map(t => t.id)).toEqual([]);
  });

  it('no longer contains a human tab (reached from the composer idle button)', () => {
    // `/human` is still a live route; the composer's primary slot opens it when
    // there is nothing to send, so a sidebar row would be a second door.
    expect(NAV_TABS.find(t => t.id === 'human')).toBeUndefined();
  });

  it('no longer contains a top-level orchestration tab (folded under Brain)', () => {
    expect(NAV_TABS.find(t => t.id === 'orchestration')).toBeUndefined();
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

  it('does not contain an intelligence or skills tab id', () => {
    expect(NAV_TABS.find(t => t.id === 'intelligence')).toBeUndefined();
    expect(NAV_TABS.find(t => t.id === 'skills')).toBeUndefined();
  });
});

describe('AVATAR_MENU_ITEMS', () => {
  it('has exactly 3 entries', () => {
    expect(AVATAR_MENU_ITEMS).toHaveLength(3);
  });

  it('has the correct ids in order', () => {
    expect(AVATAR_MENU_ITEMS.map(i => i.id)).toEqual(['account', 'invites', 'wallet']);
  });

  it('no longer offers rewards (it is a primary nav destination now)', () => {
    expect(AVATAR_MENU_ITEMS.find(i => i.id === 'rewards')).toBeUndefined();
  });

  it('invites is cloudOnly; account and wallet are not', () => {
    const cloudOnly = AVATAR_MENU_ITEMS.filter(i => i.cloudOnly).map(i => i.id);
    expect(cloudOnly).toEqual(['invites']);
  });

  it('keeps every account-menu destination inside the app', () => {
    const openUrlItems = AVATAR_MENU_ITEMS.filter(i => i.kind === 'openUrl').map(i => i.id);
    expect(openUrlItems).toEqual([]);
  });
});
