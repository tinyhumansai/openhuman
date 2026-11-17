/**
 * The Memory page's chip ids (`?brain=<chip>`) and the mapping that keeps old
 * deep links working. Shared by the page and the `/brain` redirect.
 */

export type MemoryChip =
  | 'engine'
  | 'migration'
  | 'ask'
  | 'explorer'
  | 'learnings'
  | 'conversations'
  | 'brain'
  | 'background'
  | 'settings';

export const MEMORY_CHIPS: readonly MemoryChip[] = [
  'engine',
  'migration',
  'ask',
  'explorer',
  'learnings',
  'conversations',
  'brain',
  'background',
  'settings',
];

/**
 * Retired chips → their current home. v1's graph and goals were ways of
 * asking what memory knows, and so was the context.md brief (its successor,
 * the memory pack, is previewed on Ask); v1's sources, sync and history and
 * v2's Documents chip all became the shared Brain.
 */
const LEGACY_CHIPS: Record<string, MemoryChip> = {
  graph: 'ask',
  goals: 'ask',
  context: 'ask',
  documents: 'brain',
  sources: 'brain',
  sync: 'brain',
  history: 'brain',
};

/** Resolve a raw `?brain=` value to a chip, or `null` when it names none. */
export function resolveMemoryChip(raw: string | null | undefined): MemoryChip | null {
  if (!raw) return null;
  if ((MEMORY_CHIPS as readonly string[]).includes(raw)) return raw as MemoryChip;
  return LEGACY_CHIPS[raw] ?? null;
}
