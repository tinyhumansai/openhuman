import { describe, expect, it } from 'vitest';

import reducer, {
  clearAll,
  markAllRead,
  markRead,
  type NotificationItem,
  notificationReceived,
  selectUnreadCount,
  setPreference,
} from '../notificationSlice';

const makeItem = (overrides: Partial<NotificationItem> = {}): NotificationItem => ({
  id: overrides.id ?? 'n1',
  category: overrides.category ?? 'messages',
  title: overrides.title ?? 'Hello',
  body: overrides.body ?? 'World',
  timestamp: overrides.timestamp ?? 1,
  read: overrides.read ?? false,
  ...overrides,
});

describe('notificationSlice', () => {
  it('notificationReceived prepends item', () => {
    const s1 = reducer(undefined, notificationReceived(makeItem({ id: 'a' })));
    const s2 = reducer(s1, notificationReceived(makeItem({ id: 'b' })));
    expect(s2.items.map(i => i.id)).toEqual(['b', 'a']);
  });

  it('notificationReceived dedupes by id', () => {
    let s = reducer(undefined, notificationReceived(makeItem({ id: 'dup', title: 'first' })));
    s = reducer(s, notificationReceived(makeItem({ id: 'dup', title: 'updated' })));
    expect(s.items).toHaveLength(1);
    expect(s.items[0].title).toBe('updated');
  });

  it('drops item when its category preference is off', () => {
    let s = reducer(undefined, setPreference({ category: 'messages', enabled: false }));
    s = reducer(s, notificationReceived(makeItem({ id: 'a', category: 'messages' })));
    expect(s.items).toHaveLength(0);
  });

  it('markRead flips a single item', () => {
    let s = reducer(undefined, notificationReceived(makeItem({ id: 'a' })));
    s = reducer(s, markRead({ id: 'a' }));
    expect(s.items[0].read).toBe(true);
  });

  it('markAllRead flips every item', () => {
    let s = reducer(undefined, notificationReceived(makeItem({ id: 'a' })));
    s = reducer(s, notificationReceived(makeItem({ id: 'b' })));
    s = reducer(s, markAllRead());
    expect(s.items.every(i => i.read)).toBe(true);
  });

  it('clearAll empties items', () => {
    let s = reducer(undefined, notificationReceived(makeItem({ id: 'a' })));
    s = reducer(s, clearAll());
    expect(s.items).toEqual([]);
  });

  it('selectUnreadCount counts unread', () => {
    const items = [
      makeItem({ id: 'a', read: false }),
      makeItem({ id: 'b', read: true }),
      makeItem({ id: 'c', read: false }),
    ];
    expect(selectUnreadCount(items)).toBe(2);
  });

  it('caps items at 200', () => {
    let s = reducer(undefined, notificationReceived(makeItem({ id: '0' })));
    for (let i = 1; i < 210; i++) {
      s = reducer(s, notificationReceived(makeItem({ id: String(i) })));
    }
    expect(s.items).toHaveLength(200);
    expect(s.items[0].id).toBe('209');
  });
});
