import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest';

import type { ActionableItem } from '../../../types/intelligence';
import { filterItems, getItemStats, groupItemsByTime } from '../utils';

// Pin the wall clock so day-boundary buckets are stable across the day and on CI.
const FIXED_NOW = new Date('2026-04-27T12:00:00.000Z');

beforeAll(() => {
  vi.useFakeTimers();
  vi.setSystemTime(FIXED_NOW);
});

afterAll(() => {
  vi.useRealTimers();
});

function makeItem(
  partial: Partial<ActionableItem> & { id: string; createdAt: Date }
): ActionableItem {
  return {
    title: partial.title ?? 'Untitled',
    description: partial.description,
    source: partial.source ?? 'system',
    priority: partial.priority ?? 'normal',
    status: partial.status ?? 'active',
    updatedAt: partial.updatedAt ?? partial.createdAt,
    actionable: partial.actionable ?? false,
    sourceLabel: partial.sourceLabel,
    expiresAt: partial.expiresAt,
    ...partial,
  } as ActionableItem;
}

const HOUR = 60 * 60 * 1000;
const DAY = 24 * HOUR;

function daysAgo(n: number): Date {
  return new Date(Date.now() - n * DAY);
}

describe('intelligence/utils — actionable item helpers (11.1.2)', () => {
  describe('groupItemsByTime', () => {
    it('buckets items into Today / Yesterday / This Week / Older', () => {
      const items = [
        makeItem({ id: 'today', createdAt: new Date() }),
        makeItem({ id: 'yest', createdAt: daysAgo(1) }),
        makeItem({ id: 'wk', createdAt: daysAgo(3) }),
        makeItem({ id: 'old', createdAt: daysAgo(30) }),
      ];

      const groups = groupItemsByTime(items);
      const labels = groups.map(g => g.label);
      expect(labels).toEqual(['Today', 'Yesterday', 'This Week', 'Older']);

      const find = (label: string) => groups.find(g => g.label === label);
      expect(find('Today')?.items.map(i => i.id)).toEqual(['today']);
      expect(find('Yesterday')?.items.map(i => i.id)).toEqual(['yest']);
      expect(find('This Week')?.items.map(i => i.id)).toEqual(['wk']);
      expect(find('Older')?.items.map(i => i.id)).toEqual(['old']);
    });

    it('omits empty buckets and orders within a bucket by priority then recency', () => {
      const items = [
        makeItem({ id: 'a', createdAt: new Date(), priority: 'normal' }),
        makeItem({ id: 'b', createdAt: new Date(Date.now() - 2 * HOUR), priority: 'critical' }),
        makeItem({ id: 'c', createdAt: new Date(Date.now() - HOUR), priority: 'critical' }),
      ];

      const groups = groupItemsByTime(items);
      expect(groups).toHaveLength(1);
      const today = groups[0];
      expect(today.label).toBe('Today');
      // Critical first; within critical, newer first; normal last.
      expect(today.items.map(i => i.id)).toEqual(['c', 'b', 'a']);
      expect(today.count).toBe(3);
    });

    it('handles an empty input as an empty group list', () => {
      expect(groupItemsByTime([])).toEqual([]);
    });
  });

  describe('filterItems', () => {
    const items = [
      makeItem({
        id: '1',
        createdAt: new Date(),
        title: 'Reply to Alice',
        source: 'email',
        priority: 'critical',
        status: 'active',
        sourceLabel: 'Gmail',
      }),
      makeItem({
        id: '2',
        createdAt: new Date(),
        title: 'Standup',
        source: 'calendar',
        priority: 'normal',
        status: 'active',
        sourceLabel: 'Calendar',
      }),
      makeItem({
        id: '3',
        createdAt: new Date(),
        title: 'Reply to Bob',
        source: 'email',
        priority: 'important',
        status: 'completed',
        sourceLabel: 'Gmail',
        description: 'Follow-up on the canary deployment',
      }),
    ];

    it('filters by source', () => {
      const out = filterItems(items, { source: 'email' });
      expect(out.map(i => i.id)).toEqual(['1', '3']);
    });

    it('filters by priority', () => {
      const out = filterItems(items, { priority: 'critical' });
      expect(out.map(i => i.id)).toEqual(['1']);
    });

    it('filters by status', () => {
      const out = filterItems(items, { status: 'completed' });
      expect(out.map(i => i.id)).toEqual(['3']);
    });

    it('filters by searchTerm across title, description, and sourceLabel', () => {
      expect(filterItems(items, { searchTerm: 'reply' }).map(i => i.id)).toEqual(['1', '3']);
      expect(filterItems(items, { searchTerm: 'canary' }).map(i => i.id)).toEqual(['3']);
      expect(filterItems(items, { searchTerm: 'gmail' }).map(i => i.id)).toEqual(['1', '3']);
    });

    it('treats "all" as a no-op for source/priority/status', () => {
      const out = filterItems(items, { source: 'all', priority: 'all', status: 'all' });
      expect(out.map(i => i.id)).toEqual(['1', '2', '3']);
    });

    it('returns no items when searchTerm matches nothing (failure path)', () => {
      const out = filterItems(items, { searchTerm: 'definitely-not-present' });
      expect(out).toEqual([]);
    });
  });

  describe('getItemStats', () => {
    it('counts totals, priorities, and sources, and flags new + expiringSoon', () => {
      const items = [
        makeItem({
          id: 'fresh',
          createdAt: new Date(Date.now() - 60 * 1000), // 1 minute ago
          priority: 'critical',
          source: 'email',
        }),
        makeItem({
          id: 'old',
          createdAt: new Date(Date.now() - 10 * DAY),
          priority: 'normal',
          source: 'calendar',
        }),
        makeItem({
          id: 'expSoon',
          createdAt: new Date(Date.now() - 2 * HOUR),
          priority: 'important',
          source: 'email',
          expiresAt: new Date(Date.now() + 6 * HOUR),
        }),
      ];

      const stats = getItemStats(items);
      expect(stats.total).toBe(3);
      expect(stats.byPriority.critical).toBe(1);
      expect(stats.byPriority.important).toBe(1);
      expect(stats.byPriority.normal).toBe(1);
      expect(stats.bySource.email).toBe(2);
      expect(stats.bySource.calendar).toBe(1);
      expect(stats.newItems).toBe(1);
      expect(stats.expiringSoon).toBe(1);
    });
  });
});
