import type { Thread } from '../../../types/thread';

function startOfLocalDay(date: Date): number {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
}

/**
 * Reserved thread label that marks a conversation as pinned. Persisted by the
 * core like any other label (`openhuman.threads_update_labels`), so a pin
 * survives restarts and follows the workspace. It never makes a thread a task
 * (`isThreadVisibleInTab` keys tasks off other labels), so pinned threads stay
 * in the General tab.
 */
export const PINNED_THREAD_LABEL = 'pinned';

export type ThreadGroupKey =
  | 'pinned'
  | 'today'
  | 'yesterday'
  | 'previous7Days'
  | 'previous30Days'
  | 'older';

export interface ThreadGroup {
  key: ThreadGroupKey;
  threads: Thread[];
}

export function isThreadPinned(thread: Thread): boolean {
  return Boolean(thread.labels?.includes(PINNED_THREAD_LABEL));
}

/** Labels with the pin toggled; every other label is kept in order. */
export function labelsWithPin(labels: readonly string[] | undefined, pinned: boolean): string[] {
  const rest = (labels ?? []).filter(label => label !== PINNED_THREAD_LABEL);
  return pinned ? [...rest, PINNED_THREAD_LABEL] : rest;
}

/** Case-insensitive title match; a blank query matches everything. */
export function threadMatchesQuery(title: string, query: string): boolean {
  const needle = query.trim().toLowerCase();
  return needle === '' || title.toLowerCase().includes(needle);
}

/** Which recency bucket a timestamp falls into, relative to `now`'s local day. */
export function recencyGroupFor(timestamp: string, now: Date): Exclude<ThreadGroupKey, 'pinned'> {
  const time = new Date(timestamp).getTime();
  // An unparseable timestamp sorts to the bottom rather than claiming "Today".
  if (Number.isNaN(time)) return 'older';
  const dayStart = (offset: number) =>
    startOfLocalDay(new Date(now.getFullYear(), now.getMonth(), now.getDate() - offset));
  if (time >= dayStart(0)) return 'today';
  if (time >= dayStart(1)) return 'yesterday';
  if (time >= dayStart(7)) return 'previous7Days';
  if (time >= dayStart(30)) return 'previous30Days';
  return 'older';
}

const GROUP_ORDER: ThreadGroupKey[] = [
  'pinned',
  'today',
  'yesterday',
  'previous7Days',
  'previous30Days',
  'older',
];

/**
 * Splits an already-sorted thread list into the sidebar's sections: pinned
 * first, then recency buckets by `lastMessageAt`. Input order is preserved
 * inside each section and empty sections are dropped.
 */
export function groupThreads(
  threads: readonly Thread[],
  now: Date,
  isPinned: (thread: Thread) => boolean = isThreadPinned
): ThreadGroup[] {
  const buckets = new Map<ThreadGroupKey, Thread[]>();
  for (const thread of threads) {
    const key = isPinned(thread) ? 'pinned' : recencyGroupFor(thread.lastMessageAt, now);
    const bucket = buckets.get(key);
    if (bucket) bucket.push(thread);
    else buckets.set(key, [thread]);
  }
  return GROUP_ORDER.filter(key => buckets.has(key)).map(key => ({
    key,
    threads: buckets.get(key) ?? [],
  }));
}

/** Last path segment of a folder, for a compact tooltip. */
export function folderBasename(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, '');
  const parts = trimmed.split(/[\\/]/);
  return parts[parts.length - 1] || trimmed || path;
}
