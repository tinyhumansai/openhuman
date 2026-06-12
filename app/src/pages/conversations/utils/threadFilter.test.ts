import { describe, expect, it } from 'vitest';

import type { Thread } from '../../../types/thread';
import {
  GENERAL_TAB_VALUE,
  isMeetingThread,
  isThreadVisibleInTab,
  MEETINGS_TAB_VALUE,
  SUBCONSCIOUS_TAB_VALUE,
  TASKS_TAB_VALUE,
} from './threadFilter';

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
  describe('General bucket', () => {
    it('keeps general and legacy work-labeled threads', () => {
      expect(isThreadVisibleInTab(thread({ labels: [GENERAL_TAB_VALUE] }), GENERAL_TAB_VALUE)).toBe(
        true
      );
      expect(isThreadVisibleInTab(thread({ labels: ['work', 'urgent'] }), GENERAL_TAB_VALUE)).toBe(
        true
      );
    });

    it('keeps unlabeled and unknown-label threads as the fallback bucket', () => {
      expect(isThreadVisibleInTab(thread({ labels: [] }), GENERAL_TAB_VALUE)).toBe(true);
      expect(isThreadVisibleInTab(thread({ labels: ['briefing'] }), GENERAL_TAB_VALUE)).toBe(true);
      expect(isThreadVisibleInTab(thread({ labels: ['notification'] }), GENERAL_TAB_VALUE)).toBe(
        true
      );
      expect(isThreadVisibleInTab(thread({ labels: ['custom'] }), GENERAL_TAB_VALUE)).toBe(true);
    });

    it('excludes threads that belong to explicit non-General buckets', () => {
      expect(
        isThreadVisibleInTab(thread({ labels: [SUBCONSCIOUS_TAB_VALUE] }), GENERAL_TAB_VALUE)
      ).toBe(false);
      expect(isThreadVisibleInTab(thread({ labels: [TASKS_TAB_VALUE] }), GENERAL_TAB_VALUE)).toBe(
        false
      );
      expect(isThreadVisibleInTab(thread({ parentThreadId: 'parent' }), GENERAL_TAB_VALUE)).toBe(
        false
      );
    });

    it('excludes meeting threads from the General bucket', () => {
      expect(
        isThreadVisibleInTab(thread({ labels: [MEETINGS_TAB_VALUE] }), GENERAL_TAB_VALUE)
      ).toBe(false);
      expect(isThreadVisibleInTab(thread({ labels: ['Meetings'] }), GENERAL_TAB_VALUE)).toBe(false);
    });
  });

  describe('Subconscious bucket', () => {
    it('keeps canonical and legacy subconscious-generated threads', () => {
      expect(
        isThreadVisibleInTab(thread({ labels: [SUBCONSCIOUS_TAB_VALUE] }), SUBCONSCIOUS_TAB_VALUE)
      ).toBe(true);
      expect(
        isThreadVisibleInTab(thread({ labels: ['from_reflection'] }), SUBCONSCIOUS_TAB_VALUE)
      ).toBe(true);
      expect(
        isThreadVisibleInTab(thread({ labels: ['subconscious_tick'] }), SUBCONSCIOUS_TAB_VALUE)
      ).toBe(true);
    });

    it('excludes ordinary and task threads', () => {
      expect(
        isThreadVisibleInTab(thread({ labels: [GENERAL_TAB_VALUE] }), SUBCONSCIOUS_TAB_VALUE)
      ).toBe(false);
      expect(
        isThreadVisibleInTab(thread({ labels: [TASKS_TAB_VALUE] }), SUBCONSCIOUS_TAB_VALUE)
      ).toBe(false);
    });
  });

  describe('Tasks bucket', () => {
    it('keeps task-board, legacy agent-task, and legacy worker-labeled threads', () => {
      expect(isThreadVisibleInTab(thread({ labels: [TASKS_TAB_VALUE] }), TASKS_TAB_VALUE)).toBe(
        true
      );
      expect(isThreadVisibleInTab(thread({ labels: ['agent-task'] }), TASKS_TAB_VALUE)).toBe(true);
      expect(isThreadVisibleInTab(thread({ labels: ['worker'] }), TASKS_TAB_VALUE)).toBe(true);
    });

    it('keeps parented worker/sub-agent threads regardless of labels', () => {
      expect(
        isThreadVisibleInTab(
          thread({ parentThreadId: 'parent', labels: [GENERAL_TAB_VALUE] }),
          TASKS_TAB_VALUE
        )
      ).toBe(true);
    });

    it('excludes ordinary and subconscious threads', () => {
      expect(isThreadVisibleInTab(thread({ labels: [GENERAL_TAB_VALUE] }), TASKS_TAB_VALUE)).toBe(
        false
      );
      expect(
        isThreadVisibleInTab(thread({ labels: [SUBCONSCIOUS_TAB_VALUE] }), TASKS_TAB_VALUE)
      ).toBe(false);
    });
  });

  describe('Meetings bucket', () => {
    it('keeps threads with canonical "meetings" label', () => {
      expect(
        isThreadVisibleInTab(thread({ labels: [MEETINGS_TAB_VALUE] }), MEETINGS_TAB_VALUE)
      ).toBe(true);
    });

    it('keeps threads with Rust-generated "Meetings" (capitalized) label', () => {
      expect(isThreadVisibleInTab(thread({ labels: ['Meetings'] }), MEETINGS_TAB_VALUE)).toBe(true);
    });

    it('excludes ordinary and task threads', () => {
      expect(
        isThreadVisibleInTab(thread({ labels: [GENERAL_TAB_VALUE] }), MEETINGS_TAB_VALUE)
      ).toBe(false);
      expect(isThreadVisibleInTab(thread({ labels: [TASKS_TAB_VALUE] }), MEETINGS_TAB_VALUE)).toBe(
        false
      );
      expect(isThreadVisibleInTab(thread({ labels: [] }), MEETINGS_TAB_VALUE)).toBe(false);
    });
  });

  describe('isMeetingThread helper', () => {
    it('recognizes canonical and capitalized meeting labels', () => {
      expect(isMeetingThread(thread({ labels: [MEETINGS_TAB_VALUE] }))).toBe(true);
      expect(isMeetingThread(thread({ labels: ['Meetings'] }))).toBe(true);
    });

    it('rejects non-meeting threads', () => {
      expect(isMeetingThread(thread({ labels: [] }))).toBe(false);
      expect(isMeetingThread(thread({ labels: [GENERAL_TAB_VALUE] }))).toBe(false);
    });
  });
});
