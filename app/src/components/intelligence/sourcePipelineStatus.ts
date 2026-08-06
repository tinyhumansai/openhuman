/**
 * Layered pipeline-health derivation for Data Sync source rows (GH-4690).
 *
 * Raw sync ≠ retrieval-ready. A source can be "synced" (its documents were
 * ingested into `mem_tree_chunks`) while the downstream retrieval pipeline
 * silently failed underneath it: embeddings were never created, spaCy/LLM
 * extraction failed, or the memory tree is degraded. Before this, Data Sync
 * showed a clean freshness badge in all those cases and the user only learned
 * the truth in Brain > Memory > Sync or the raw logs.
 *
 * This module folds two signals the core already exposes into one per-row
 * verdict so the row can honestly say "Ingested only" instead of "synced":
 *
 * 1. **Per-source, precise** — `SourceStatus.chunks_pending` is the SQL count
 *    of this source's chunks whose `embedding IS NULL` (see
 *    `memory_sources/status.rs`). `> 0` in a settled state means those chunks
 *    were stored WITHOUT vectors → semantic search can't reach them.
 * 2. **Global pipeline health** — `memory_tree_pipeline_status` (the same RPC
 *    that drives the Brain > Memory > Sync "Degraded" panel) carries the
 *    process-wide `degraded` snapshot + `first_blocking_cause`. Embedding /
 *    extraction / tree-degraded causes there are attributed to the whole
 *    pipeline, so we surface them on rows that actually contributed chunks.
 *
 * The function is pure so it can be unit-tested exhaustively without a DOM.
 */
import type { SourceStatus } from '../../services/memorySourcesService';
import type { MemoryTreePipelineStatus } from '../../utils/tauriCommands/memoryTree';

/** One failed layer of the post-ingest pipeline, in severity-display order. */
export type SourcePipelineIssueKind =
  | 'stored_without_vectors'
  | 'extraction_failed'
  | 'tree_degraded';

/** Coarse retrieval-readiness state for a single source row. */
export type SourcePipelineState = 'none' | 'retrieval_ready' | 'ingested_only';

export interface SourcePipelineHealth {
  /**
   * `none` — nothing ingested yet (no badge changes; row renders as before).
   * `retrieval_ready` — chunks synced AND no downstream failure (clean state).
   * `ingested_only` — chunks synced but ≥1 downstream layer failed (warn).
   */
  state: SourcePipelineState;
  /** The failed layers, deduped, in display order. Empty unless `ingested_only`. */
  issues: SourcePipelineIssueKind[];
  /**
   * True when the embeddings failure is attributable to a missing backend
   * session / auth (the "No backend session for cloud embeddings" case). Drives
   * the "Sign in to enable" affordance — only meaningful with
   * `stored_without_vectors`.
   */
  authRelated: boolean;
}

/**
 * Compute the layered pipeline verdict for one source row.
 *
 * `status` is this source's `memory_sources_status_list` entry; `pipeline` is
 * the global `memory_tree_pipeline_status` snapshot (may be `null` when that
 * RPC hasn't resolved / failed — the per-source signal still stands on its own).
 */
export function deriveSourcePipelineHealth(
  status: SourceStatus | null,
  pipeline: MemoryTreePipelineStatus | null
): SourcePipelineHealth {
  const synced = status?.chunks_synced ?? 0;

  // Nothing ingested yet → don't invent a warning. The row keeps its existing
  // (empty / mid-sync) rendering.
  if (synced <= 0) {
    return { state: 'none', issues: [], authRelated: false };
  }

  const degraded = pipeline?.degraded;
  const causeCode = pipeline?.first_blocking_cause?.code ?? degraded?.cause?.code ?? null;

  const issues: SourcePipelineIssueKind[] = [];

  // Layer 1 — embeddings. Precise per-source signal (chunks with NULL
  // embedding) OR the global "semantic recall degraded" latch (no usable
  // embeddings provider). Either means this source's chunks aren't vector-searchable.
  const storedWithoutVectors =
    (status?.chunks_pending ?? 0) > 0 || degraded?.semantic_recall === true;
  if (storedWithoutVectors) {
    issues.push('stored_without_vectors');
  }

  // Layer 2 — extraction. `degraded.structure` exists but has no production
  // producer today (test-only), so the live signal is the typed
  // `extraction_timeout` blocking cause ("the memory extraction model is
  // timing out"). Honour both.
  const extractionFailed = degraded?.structure === true || causeCode === 'extraction_timeout';
  if (extractionFailed) {
    issues.push('extraction_failed');
  }

  // Layer 3 — memory tree. A global degraded/error status that isn't already
  // explained by a more specific layer above (e.g. storage-degraded, a failed
  // job). Retrieval for every synced source may return stale results.
  const treeDegraded =
    (pipeline?.status === 'degraded' || pipeline?.status === 'error') &&
    !storedWithoutVectors &&
    !extractionFailed;
  if (treeDegraded) {
    issues.push('tree_degraded');
  }

  const authRelated = storedWithoutVectors && causeCode === 'auth_missing';

  return { state: issues.length > 0 ? 'ingested_only' : 'retrieval_ready', issues, authRelated };
}

/** i18n key for a given issue's row warning message. */
export function pipelineIssueMessageKey(kind: SourcePipelineIssueKind): string {
  switch (kind) {
    case 'stored_without_vectors':
      return 'sync.pipeline.storedWithoutVectors';
    case 'extraction_failed':
      return 'sync.pipeline.extractionFailed';
    case 'tree_degraded':
      return 'sync.pipeline.treeDegraded';
  }
}
