/**
 * Shared `GraphRelation` test factory for the Knowledge Freshness suites.
 *
 * Keeps the relation shape (and the `NOW` / `DAY` anchors) in one place so the
 * pure-engine tests (`memoryFreshness.test.ts`) and the API-facade tests
 * (`memoryFreshnessApi.test.ts`) stay in sync as `GraphRelation` evolves.
 */
import type { GraphRelation } from '../utils/tauriCommands/memory';

/** Fixed reference instant (epoch seconds) so every scored report is deterministic. */
export const NOW = 1_700_000_000;
/** Seconds in a day — `agoDays` is multiplied by this to age a relation. */
export const DAY = 86400;

/**
 * Build a `GraphRelation` aged `agoDays` days before {@link NOW}. The unused
 * graph fields (attrs / ids) are filled with inert defaults; callers override
 * via spread when a specific field matters.
 */
export function rel(
  subject: string,
  object: string,
  agoDays: number,
  evidenceCount = 1,
  predicate = 'p',
  namespace = 'n'
): GraphRelation {
  return {
    namespace,
    subject,
    predicate,
    object,
    attrs: {},
    updatedAt: NOW - agoDays * DAY,
    evidenceCount,
    orderIndex: null,
    documentIds: [],
    chunkIds: [],
  };
}
