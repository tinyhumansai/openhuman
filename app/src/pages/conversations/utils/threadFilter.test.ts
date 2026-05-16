import { describe, expect, it } from 'vitest';

import type { Thread } from '../../../types/thread';
import { isThreadVisibleInTab, WORKERS_TAB_VALUE } from './threadFilter';

// Issue #1624: this is the pure rule that backs the sidebar
// `filteredThreads` memo + the `Workers` tab. The tests pin both halves
// of the contract so a future tweak to one branch can never silently
// regress the other:
//   1. Default tabs hide worker threads (parentThreadId set).
//   2. The `Workers` sentinel tab inverts that filter.

function thread(overrides: Partial<Thread>): Thread {
  return {
    id: overrides.id ?? 't',
    title: overrides.title ?? 'Untitled',
    chatId: null,
    isActive: true,
    messageCount: 0,
    lastMessageAt: '2026-05-15T10:00:00Z',
    createdAt: '2026-05-15T09:00:00Z',
    parentThreadId: overrides.parentThreadId,
    labels: overrides.labels ?? [],
  };
}

describe('isThreadVisibleInTab', () => {
  describe('default `all` tab', () => {
    it('keeps top-level conversations', () => {
      expect(isThreadVisibleInTab(thread({ id: 'top' }), 'all')).toBe(true);
    });

    it('hides worker threads (parentThreadId set)', () => {
      expect(isThreadVisibleInTab(thread({ id: 'w', parentThreadId: 'p' }), 'all')).toBe(false);
    });
  });

  describe('label-scoped tabs (work, briefing, notification, …)', () => {
    it('keeps a non-worker thread that carries the matching label', () => {
      const t = thread({ id: 'a', labels: ['work', 'urgent'] });
      expect(isThreadVisibleInTab(t, 'work')).toBe(true);
    });

    it('drops a non-worker thread that does not carry the matching label', () => {
      const t = thread({ id: 'a', labels: ['briefing'] });
      expect(isThreadVisibleInTab(t, 'work')).toBe(false);
    });

    it('still hides worker threads even when the label would otherwise match', () => {
      const t = thread({ id: 'w', parentThreadId: 'p', labels: ['work'] });
      expect(isThreadVisibleInTab(t, 'work')).toBe(false);
    });

    it('treats threads with no labels array as not matching', () => {
      const t = thread({ id: 'a' });
      expect(isThreadVisibleInTab(t, 'briefing')).toBe(false);
    });
  });

  describe('Workers tab (intentional surface for sub-agent work)', () => {
    it('uses the shared sentinel value', () => {
      // Lock the constant — Conversations.tsx wires the same string into
      // the sidebar tab definition, so a rename without updating both
      // sides would silently break the surface.
      expect(WORKERS_TAB_VALUE).toBe('workers');
    });

    it('keeps a worker thread', () => {
      const t = thread({ id: 'w', parentThreadId: 'p' });
      expect(isThreadVisibleInTab(t, WORKERS_TAB_VALUE)).toBe(true);
    });

    it('hides top-level conversations', () => {
      const t = thread({ id: 'top' });
      expect(isThreadVisibleInTab(t, WORKERS_TAB_VALUE)).toBe(false);
    });

    it("keeps a worker thread regardless of the worker thread's own labels", () => {
      const t = thread({ id: 'w', parentThreadId: 'p', labels: ['work'] });
      expect(isThreadVisibleInTab(t, WORKERS_TAB_VALUE)).toBe(true);
    });
  });
});
