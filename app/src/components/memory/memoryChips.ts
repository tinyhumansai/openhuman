/**
 * The Memory page's chip ids (`?brain=<chip>`) and the mapping that keeps old
 * v1 deep links working. Shared by the page and the `/brain` redirect.
 */

export type MemoryChip = 'engine' | 'ask' | 'learnings' | 'conversations' | 'documents' | 'context';

export const MEMORY_CHIPS: readonly MemoryChip[] = [
  'engine',
  'ask',
  'learnings',
  'conversations',
  'documents',
  'context',
];

/**
 * v1 sub-tabs → their v2 home: the graph and goals were ways of asking what
 * memory knows; sources, sync and history were all about synced documents.
 */
const LEGACY_CHIPS: Record<string, MemoryChip> = {
  graph: 'ask',
  goals: 'ask',
  sources: 'documents',
  sync: 'documents',
  history: 'documents',
};

/** Resolve a raw `?brain=` value to a chip, or `null` when it names none. */
export function resolveMemoryChip(raw: string | null | undefined): MemoryChip | null {
  if (!raw) return null;
  if ((MEMORY_CHIPS as readonly string[]).includes(raw)) return raw as MemoryChip;
  return LEGACY_CHIPS[raw] ?? null;
}
