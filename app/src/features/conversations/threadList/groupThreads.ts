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

type RecencyGroupKey =
  | 'pinned'
  | 'today'
  | 'yesterday'
  | 'previous7Days'
  | 'previous30Days'
  | 'older';

/** Workspace-specific conversation sections use the full working-directory identity. */
export type ThreadGroupKey = RecencyGroupKey | `project:${string}`;

export interface ThreadGroup {
  key: ThreadGroupKey;
  /** Project section heading; recency sections use their localized labels. */
  label?: string;
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
export function recencyGroupFor(timestamp: string, now: Date): Exclude<RecencyGroupKey, 'pinned'> {
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

const GROUP_ORDER: RecencyGroupKey[] = [
  'pinned',
  'today',
  'yesterday',
  'previous7Days',
  'previous30Days',
  'older',
];

/** Windows paths are case-insensitive, while Unix paths preserve case. */
function projectDirectoryKey(path: string): string {
  const windowsPath = /^(?:[a-z]:[\\/]|\\\\|\/\/)/i.test(path);
  const normalized = windowsPath ? path.replace(/\//g, '\\').toLowerCase() : path;
  return (
    (windowsPath ? normalized.replace(/\\+$/, '') : normalized.replace(/\/+$/, '')) || normalized
  );
}

/**
 * Groups pinned conversations first, then working directories, then recency.
 * The full path (not the final folder name) determines project identity.
 * Input order is preserved inside each section.
 */
export function groupThreads(
  threads: readonly Thread[],
  now: Date,
  isPinned: (thread: Thread) => boolean = isThreadPinned
): ThreadGroup[] {
  const buckets = new Map<ThreadGroupKey, ThreadGroup>();
  for (const thread of threads) {
    const projectPath = thread.actionDir?.trim();
    const project = projectPath ? folderBasename(projectPath) : null;
    let key: ThreadGroupKey;
    let label: string | undefined;
    if (isPinned(thread)) {
      key = 'pinned';
    } else if (projectPath && project) {
      key = `project:${projectDirectoryKey(projectPath)}`;
      label = project;
    } else {
      key = recencyGroupFor(thread.lastMessageAt, now);
    }
    const bucket = buckets.get(key);
    if (bucket) bucket.threads.push(thread);
    else buckets.set(key, { key, ...(label ? { label } : {}), threads: [thread] });
  }
  const orderedDynamic = [...buckets.values()]
    .filter(group => group.key.startsWith('project:'))
    .sort((a, b) => {
      return (
        (a.label ?? '').localeCompare(b.label ?? '', undefined, { sensitivity: 'base' }) ||
        (a.key < b.key ? -1 : a.key > b.key ? 1 : 0)
      );
    });
  const orderedRecency = GROUP_ORDER.flatMap(key => {
    const group = buckets.get(key);
    return group ? [group] : [];
  });
  const pinned = buckets.get('pinned');
  const recencyStart = pinned ? 1 : 0;
  return [...(pinned ? [pinned] : []), ...orderedDynamic, ...orderedRecency.slice(recencyStart)];
}

/** Last path segment of a folder, for a compact tooltip. */
export function folderBasename(path: string): string {
  const windowsPath = /^(?:[a-z]:[\\/]|\\\\|\/\/)/i.test(path);
  const trimmed = windowsPath ? path.replace(/[\\/]+$/, '') : path.replace(/\/+$/, '');
  const parts = trimmed.split(windowsPath ? /[\\/]/ : /\//);
  return parts[parts.length - 1] || trimmed || path;
}
