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
 * This module folds three signals the core already exposes into one per-row
 * verdict so the row can honestly say "Ingested only" instead of "synced":
 *
 * 1. **Per-source, precise** — `SourceStatus.chunks_pending` is the SQL count
 *    of this source's chunks with no vector at the active embedding signature
 *    (see `memory_sources/status.rs`). `> 0` means semantic search cannot
 *    reach those chunks *yet*.
 * 2. **Global pipeline health** — `memory_tree_pipeline_status` (the same RPC
 *    that drives the Brain > Memory > Sync "Degraded" panel) carries the
 *    process-wide `degraded` snapshot + `first_blocking_cause`. Embedding /
 *    extraction / tree-degraded causes there are attributed to the whole
 *    pipeline, so we surface them on rows that actually contributed chunks.
 * 3. **Embed work in flight** — `memory_tree_memory_backfill_status` says
 *    whether a re-embed chain still has rows to process. Since tinymemory
 *    1.14 chunks get their vectors from that chain, not inline at ingest, so
 *    every sizeable sync leaves a backlog for a few minutes
 *    (openhuman#6025). A backlog that is being drained is *pending*, not
 *    *failed*: the row shows a neutral note and keeps its freshness pill. The
 *    amber warning is reserved for a backlog nothing will drain: the global
 *    recall latch, an embeddings-family blocking cause, a paused scheduler
 *    gate, or pending chunks with no chain queued. The source's freshness
 *    stands in only when that snapshot is unavailable.
 *
 * The function is pure so it can be unit-tested exhaustively without a DOM.
 */
import type { SourceStatus } from '../../services/memorySourcesService';
import type {
  BackfillStatus,
  MemoryTreePipelineStatus,
} from '../../utils/tauriCommands/memoryTree';

/** One failed layer of the post-ingest pipeline, in severity-display order. */
export type SourcePipelineIssueKind =
  | 'stored_without_vectors'
  | 'extraction_failed'
  | 'tree_degraded';

/** Coarse retrieval-readiness state for a single source row. */
export type SourcePipelineState = 'none' | 'retrieval_ready' | 'vectors_pending' | 'ingested_only';

export interface SourcePipelineHealth {
  /**
   * `none` — nothing ingested yet (no badge changes; row renders as before).
   * `retrieval_ready` — chunks synced AND no downstream failure (clean state).
   * `vectors_pending` — chunks synced, some still waiting for an embed
   *   backfill that is provably working on them (neutral note, no warning).
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
  /**
   * True when this source has chunks waiting for vectors AND the backlog is
   * being drained (the soft state). Carried separately from `state` so the
   * row can still show the neutral note beside another layer's warning.
   */
  vectorsPending: boolean;
}

/**
 * Blocking causes that mean the embeddings provider itself cannot write
 * vectors, so a pending backlog is not going to drain on its own. Anything
 * else (`transient`, `extraction_timeout`, …) leaves the backfill able to
 * finish, and the row must not read the backlog as a failure.
 */
const EMBEDDINGS_BLOCKING_CAUSES: ReadonlySet<string> = new Set([
  'budget_exhausted',
  'auth_missing',
  'auth_invalid',
  'embeddings_unconfigured',
  'embedding_dim_mismatch',
  'local_model_unavailable',
]);

/**
 * Compute the layered pipeline verdict for one source row.
 *
 * `status` is this source's `memory_sources_status_list` entry; `pipeline` is
 * the global `memory_tree_pipeline_status` snapshot (may be `null` when that
 * RPC hasn't resolved / failed — the per-source signal still stands on its
 * own); `backfill` is the global `memory_tree_memory_backfill_status` snapshot
 * (`null` when unavailable — the row then falls back to the source's own
 * freshness to decide whether a backlog is still moving).
 */
export function deriveSourcePipelineHealth(
  status: SourceStatus | null,
  pipeline: MemoryTreePipelineStatus | null,
  backfill: BackfillStatus | null = null
): SourcePipelineHealth {
  const synced = status?.chunks_synced ?? 0;

  // Nothing ingested yet → don't invent a warning. The row keeps its existing
  // (empty / mid-sync) rendering.
  if (synced <= 0) {
    return { state: 'none', issues: [], authRelated: false, vectorsPending: false };
  }

  const degraded = pipeline?.degraded;
  const causeCode = pipeline?.first_blocking_cause?.code ?? degraded?.cause?.code ?? null;

  const issues: SourcePipelineIssueKind[] = [];

  // Layer 1 — embeddings. `chunks_pending` alone is ambiguous: right after a
  // sync it is the backfill's to-do list, minutes later with nothing queued
  // it is a hole in recall. Three signals settle which (openhuman#6025):
  //
  // - the global "semantic recall degraded" latch (no usable provider): hard;
  // - an embeddings-family blocking cause: the provider cannot write vectors,
  //   so the backlog will not drain: hard;
  // - a paused scheduler: the configured `off` mode (`is_paused` /
  //   `status === 'paused'`) or the gate's live policy (`gate_paused`: on
  //   battery, CPU pressure, signed out). The chain may be armed and ingest
  //   may still be writing, but no worker will embed anything until the gate
  //   lifts, so "shortly" would be a promise nobody is keeping. Pending
  //   chunks are hard until then;
  // - a stalled queue (`queue_stalled`: eligible work waiting six hours with
  //   nothing settling, the #5324 verdict): a stalled `reembed_backfill` row
  //   still keeps the snapshot `in_progress`, but the core has ruled that
  //   nothing is draining. Hard;
  // - otherwise the engine's own word decides: pending chunks are soft while
  //   `backfill.in_progress` reports a re-embed chain with rows to process,
  //   and hard when that snapshot says no chain is queued. Only when the
  //   snapshot is unavailable (RPC failed, older core) does the source's
  //   freshness stand in: a chunk inside the core's `recent` window (≤ 5 min)
  //   suggests ingest is still writing and the chain arms on the next extract
  //   admit. A fallback, not evidence of a worker — `last_chunk_at_ms` is the
  //   content's own time (an email's sent date), which is why it never
  //   outranks an explicit "no chain queued".
  //
  // `backfill.in_progress` is process-wide on purpose, not a per-source
  // signal to be narrowed: the chain it reports is signature-wide — each
  // batch takes any chunk lacking a vector at the active signature, whoever
  // ingested it — so a running chain is draining THIS source's backlog too.
  // (A chunk the chain skipped for good is tombstoned and no longer counts as
  // pending, so it cannot hide behind the flag.)
  const pending = status?.chunks_pending ?? 0;
  const recallLatched = degraded?.semantic_recall === true;
  const embeddingsBlocked = causeCode !== null && EMBEDDINGS_BLOCKING_CAUSES.has(causeCode);
  const paused =
    pipeline?.is_paused === true || pipeline?.status === 'paused' || pipeline?.gate_paused === true;
  const stalled = pipeline?.queue_stalled === true;
  const draining =
    !paused &&
    !stalled &&
    (backfill
      ? backfill.in_progress === true
      : status?.freshness === 'active' || status?.freshness === 'recent');
  const storedWithoutVectors = recallLatched || (pending > 0 && (embeddingsBlocked || !draining));
  if (storedWithoutVectors) {
    issues.push('stored_without_vectors');
  }
  const vectorsPending = !storedWithoutVectors && pending > 0;

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

  const state: SourcePipelineState =
    issues.length > 0 ? 'ingested_only' : vectorsPending ? 'vectors_pending' : 'retrieval_ready';

  return { state, issues, authRelated, vectorsPending };
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

/** i18n key for the neutral "waiting for vectors" note (`{count}` placeholder). */
export const VECTORS_PENDING_MESSAGE_KEY = 'sync.pipeline.vectorsPending';
