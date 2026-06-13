/**
 * Persist-layer filter for `chatRuntime.artifactsByThread` (#3024).
 *
 * Only `status === 'ready'` snapshots survive a write to storage:
 *
 *  - `in_progress` would resurrect "Generating…" placeholders on cold
 *    boot even though no producer task is alive — a misleading
 *    forever-spinner state.
 *  - `failed` carries a producer-supplied error message that may
 *    reference a session-bound run (timeout, engine internal error),
 *    which is irrelevant after a restart. Letting the failed card
 *    persist would suggest a permanent failure rather than a
 *    transient one.
 *
 * Threads with zero ready snapshots are dropped from the persisted
 * map so the storage layer stays compact across cold reboots.
 *
 * Extracted from `store/index.ts`'s redux-persist `createTransform`
 * so the pure data behaviour is unit-testable without instantiating
 * the persist machinery.
 */
import type { ArtifactSnapshot } from './chatRuntimeSlice';

export type ArtifactsByThread = Record<string, ArtifactSnapshot[]>;

/**
 * Filter the in-memory `artifactsByThread` map to the subset that
 * should be written to storage. Pure — input is not mutated.
 *
 * @param inbound - the live slice's `artifactsByThread` map, or
 *   `undefined` when the slice has never been written.
 * @returns a fresh map containing only `status === 'ready'` entries,
 *   with empty buckets removed.
 */
export function filterArtifactsForPersist(
  inbound: ArtifactsByThread | undefined
): ArtifactsByThread {
  if (!inbound) return {};
  const filtered: ArtifactsByThread = {};
  for (const [threadId, list] of Object.entries(inbound)) {
    const readyOnly = list.filter(entry => entry.status === 'ready');
    if (readyOnly.length > 0) {
      filtered[threadId] = readyOnly;
    }
  }
  return filtered;
}

/**
 * Rehydrate the persisted map back into the live slice shape.
 *
 * Storage was already filtered on write, so this is a trust-but-defend
 * step: missing/nullish inputs collapse to an empty map rather than
 * propagating `undefined` through the rehydration pipeline.
 */
export function rehydrateArtifactsFromPersist(
  outbound: ArtifactsByThread | undefined
): ArtifactsByThread {
  return outbound ?? {};
}
