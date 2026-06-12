import type { Thread } from '../../../types/thread';

export const GENERAL_TAB_VALUE = 'general';
export const SUBCONSCIOUS_TAB_VALUE = 'subconscious';
export const TASKS_TAB_VALUE = 'tasks';
export const MEETINGS_TAB_VALUE = 'meetings';
export const LEGACY_GENERAL_LABEL = 'work';
export const LEGACY_SUBCONSCIOUS_LABELS = ['from_reflection', 'subconscious_tick'];
export const LEGACY_TASK_LABELS = ['agent-task', 'worker'];
/** Canonical label applied to meeting transcript threads by the Rust core. */
const MEETINGS_LABEL = 'Meetings';

function hasAnyLabel(thread: Thread, labels: readonly string[]): boolean {
  return Boolean(thread.labels?.some(label => labels.includes(label)));
}

function isSubconsciousThread(thread: Thread): boolean {
  return hasAnyLabel(thread, [SUBCONSCIOUS_TAB_VALUE, ...LEGACY_SUBCONSCIOUS_LABELS]);
}

export function isTaskThread(thread: Thread): boolean {
  return Boolean(
    thread.parentThreadId || hasAnyLabel(thread, [TASKS_TAB_VALUE, ...LEGACY_TASK_LABELS])
  );
}

export function isMeetingThread(thread: Thread): boolean {
  return hasAnyLabel(thread, [MEETINGS_TAB_VALUE, MEETINGS_LABEL]);
}

/**
 * Pure, side-effect-free thread filter shared between
 * `Conversations.tsx` (which renders the sidebar list) and the test
 * suite. Keeping it free of React state means a future change to the
 * filter rule lands in one place with explicit unit coverage instead
 * of a buried `useMemo` body.
 *
 * Rules:
 *   - Tasks includes task-board threads plus legacy worker/sub-agent threads.
 *   - Subconscious includes new and legacy reflection/tick-generated threads.
 *   - General is the fallback bucket for everything else, including legacy
 *     `work`, unknown labels, and unlabeled historical threads.
 */
export function isThreadVisibleInTab(thread: Thread, selectedLabel: string): boolean {
  const isSubconscious = isSubconsciousThread(thread);
  const isTask = isTaskThread(thread);
  const isMeeting = isMeetingThread(thread);
  if (selectedLabel === SUBCONSCIOUS_TAB_VALUE) return isSubconscious;
  if (selectedLabel === TASKS_TAB_VALUE) return isTask;
  if (selectedLabel === MEETINGS_TAB_VALUE) return isMeeting;
  if (selectedLabel === GENERAL_TAB_VALUE) {
    return !isSubconscious && !isTask && !isMeeting;
  }
  return Boolean(thread.labels?.includes(selectedLabel));
}
