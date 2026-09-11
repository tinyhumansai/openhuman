/**
 * Sync-progress types shared between `MemorySourcesRegistry` (which owns the
 * in-flight state) and `MemorySourceRow` (which renders it). Split out so
 * neither file needs to import the other for a type.
 */

/** Live progress for a sync in flight, driven by MemorySyncStageChanged events. */
export interface SyncProgress {
  stage: string;
  detail: string | null;
  percent: number | null;
}

/**
 * Terminal outcome of a sync run, shown on the row after the `completed` or
 * `failed` stage event arrives (#3295). Persists until the next sync starts,
 * so a no-op ("0 new items") or failed sync leaves visible confirmation
 * instead of the indicator silently vanishing.
 */
/**
 * Why a completed run stopped short, parsed from the remainder the core writes
 * after the item count. `more_pending`: the per-run cap or the day's budget
 * left more to read — click Sync again. `budget_spent`: the day's provider
 * request budget is gone, so nothing more arrives until tomorrow; with a zero
 * count that is the opposite of "Up to date", which is what the row used to
 * say.
 */
export type SyncNote = 'more_pending' | 'budget_spent';

export interface SyncResult {
  kind: 'success' | 'failed';
  /** New items ingested (success only); null when the count is unknown. */
  items: number | null;
  /** Human-readable failure reason (failed only). */
  reason: string | null;
  /** Why the run stopped short (success only); null when it did not. */
  note: SyncNote | null;
}

/**
 * Per-stage fallback percentages so the progress bar always advances even
 * when no numeric "N/M" ratio is present in the detail string (RC#4, #3295).
 */
export const STAGE_FALLBACK_PERCENT: Record<string, number> = {
  requested: 2,
  fetching: 5,
  stored: 15,
  queued: 25,
  ingesting: 40,
  completed: 100,
};
