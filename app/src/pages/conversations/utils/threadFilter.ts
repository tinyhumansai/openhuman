import type { Thread } from '../../../types/thread';

/**
 * Sentinel label-tab value that surfaces only worker / sub-agent threads
 * (any thread whose `parentThreadId` is set). Issue #1624: the
 * `Workers` tab in the conversation sidebar is the deliberate UI surface
 * for background sub-agent activity — picking it inverts the default
 * `parentThreadId` filter so users can scan, open, and inspect worker
 * transcripts that the main `All` view intentionally hides.
 *
 * Lives at module scope so the sidebar tab definition, the filter
 * function, and the Vitest specs all reference the same string and a
 * future rename can never silently desync the three.
 */
export const WORKERS_TAB_VALUE = 'workers';

/**
 * Pure, side-effect-free thread filter shared between
 * `Conversations.tsx` (which renders the sidebar list) and the test
 * suite. Keeping it free of React state means a future change to the
 * filter rule lands in one place with explicit unit coverage instead
 * of a buried `useMemo` body.
 *
 * Rules (issue #1624):
 *   - When `selectedLabel === WORKERS_TAB_VALUE`, return only worker
 *     threads (those with `parentThreadId`). The Workers tab is the
 *     intentional surface for background sub-agent activity.
 *   - Otherwise, hide every worker thread so the main sidebar stays
 *     dominated by user-initiated conversations and isn't polluted by
 *     orchestrator-spawned background work.
 *   - Within the non-Workers tabs, `selectedLabel === 'all'` keeps every
 *     non-worker thread; any other value scopes by the existing thread
 *     `labels[]` array (`work`, `briefing`, `notification`, …).
 */
export function isThreadVisibleInTab(thread: Thread, selectedLabel: string): boolean {
  const isWorker = Boolean(thread.parentThreadId);
  if (selectedLabel === WORKERS_TAB_VALUE) return isWorker;
  if (isWorker) return false;
  if (selectedLabel === 'all') return true;
  return Boolean(thread.labels?.includes(selectedLabel));
}
