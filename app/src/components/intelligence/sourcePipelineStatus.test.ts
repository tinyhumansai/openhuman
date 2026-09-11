/**
 * Unit tests for the layered Data Sync pipeline-health derivation (GH-4690).
 * Covers each warning layer plus the healthy no-regression case.
 */
import { describe, expect, it } from 'vitest';

import type { SourceStatus } from '../../services/memorySourcesService';
import type {
  BackfillStatus,
  MemoryTreePipelineStatus,
} from '../../utils/tauriCommands/memoryTree';
import {
  deriveSourcePipelineHealth,
  pipelineIssueMessageKey,
  type SourcePipelineIssueKind,
} from './sourcePipelineStatus';

function makeStatus(overrides: Partial<SourceStatus> = {}): SourceStatus {
  return {
    source_id: 'src_1',
    chunks_synced: 5,
    chunks_pending: 0,
    last_chunk_at_ms: 1_000,
    freshness: 'recent',
    ...overrides,
  };
}

function makePipeline(overrides: Partial<MemoryTreePipelineStatus> = {}): MemoryTreePipelineStatus {
  return {
    status: 'running',
    reason: null,
    last_sync_ms: 1_000,
    total_chunks: 5,
    wiki_size_bytes: 0,
    pipeline_jobs: { ready: 0, running: 0, failed: 0 },
    is_syncing: false,
    is_paused: false,
    ...overrides,
  };
}

/** The global embed-backfill snapshot (`memory_tree_memory_backfill_status`). */
function makeBackfill(overrides: Partial<BackfillStatus> = {}): BackfillStatus {
  return { in_progress: false, pending_jobs: 0, ...overrides };
}

describe('deriveSourcePipelineHealth', () => {
  it('returns none when nothing has been ingested yet', () => {
    const h = deriveSourcePipelineHealth(makeStatus({ chunks_synced: 0 }), makePipeline());
    expect(h.state).toBe('none');
    expect(h.issues).toEqual([]);
  });

  it('returns none when status is null (pre-load)', () => {
    const h = deriveSourcePipelineHealth(null, makePipeline());
    expect(h.state).toBe('none');
  });

  // -- No regression: fully healthy sync stays clean -------------------------
  it('reports retrieval_ready when everything is healthy', () => {
    const h = deriveSourcePipelineHealth(
      makeStatus({ chunks_synced: 5, chunks_pending: 0 }),
      makePipeline({ status: 'running' })
    );
    expect(h.state).toBe('retrieval_ready');
    expect(h.issues).toEqual([]);
    expect(h.authRelated).toBe(false);
  });

  it('stays retrieval_ready when the pipeline snapshot is missing but chunks are embedded', () => {
    const h = deriveSourcePipelineHealth(makeStatus({ chunks_pending: 0 }), null);
    expect(h.state).toBe('retrieval_ready');
    expect(h.issues).toEqual([]);
  });

  // -- Layer 1: embeddings ---------------------------------------------------
  it('flags stored_without_vectors from per-source pending chunks alone', () => {
    // The exact issue repro: "1 chunk / 1 pending" with no pipeline snapshot,
    // and nothing draining it: no backfill snapshot, newest chunk long idle.
    const h = deriveSourcePipelineHealth(
      makeStatus({ chunks_synced: 1, chunks_pending: 1, freshness: 'idle' }),
      null
    );
    expect(h.state).toBe('ingested_only');
    expect(h.issues).toContain('stored_without_vectors');
  });

  it('flags stored_without_vectors from the global semantic_recall latch', () => {
    const h = deriveSourcePipelineHealth(
      makeStatus({ chunks_pending: 0 }),
      makePipeline({ status: 'degraded', degraded: { semantic_recall: true, structure: false } })
    );
    expect(h.issues).toContain('stored_without_vectors');
    // A recall-degraded state must NOT also add the generic tree_degraded noise.
    expect(h.issues).not.toContain('tree_degraded');
  });

  it('marks authRelated when the blocking cause is a missing backend session', () => {
    const h = deriveSourcePipelineHealth(
      makeStatus({ chunks_pending: 3 }),
      makePipeline({
        status: 'error',
        first_blocking_cause: {
          code: 'auth_missing',
          class: 'unrecoverable',
          remediation_key: 'memory.health.remediation.auth_missing',
        },
      })
    );
    expect(h.issues).toContain('stored_without_vectors');
    expect(h.authRelated).toBe(true);
  });

  it('does not mark authRelated for a non-auth embeddings cause', () => {
    const h = deriveSourcePipelineHealth(
      makeStatus({ chunks_pending: 3 }),
      makePipeline({
        status: 'error',
        first_blocking_cause: {
          code: 'embeddings_unconfigured',
          class: 'unrecoverable',
          remediation_key: 'memory.health.remediation.embeddings_unconfigured',
        },
      })
    );
    expect(h.authRelated).toBe(false);
  });

  // -- Layer 2: extraction ---------------------------------------------------
  it('flags extraction_failed from an extraction_timeout blocking cause', () => {
    const h = deriveSourcePipelineHealth(
      makeStatus({ chunks_pending: 0 }),
      makePipeline({
        status: 'degraded',
        first_blocking_cause: {
          code: 'extraction_timeout',
          class: 'transient',
          remediation_key: 'memory.health.remediation.extraction_timeout',
        },
      })
    );
    expect(h.state).toBe('ingested_only');
    expect(h.issues).toContain('extraction_failed');
  });

  it('flags extraction_failed from the structure degraded flag', () => {
    const h = deriveSourcePipelineHealth(
      makeStatus({ chunks_pending: 0 }),
      makePipeline({ status: 'degraded', degraded: { semantic_recall: false, structure: true } })
    );
    expect(h.issues).toContain('extraction_failed');
  });

  // -- Layer 3: memory tree --------------------------------------------------
  it('flags tree_degraded for a generic degraded status with no specific layer', () => {
    const h = deriveSourcePipelineHealth(
      makeStatus({ chunks_pending: 0 }),
      makePipeline({ status: 'degraded' })
    );
    expect(h.state).toBe('ingested_only');
    expect(h.issues).toEqual(['tree_degraded']);
  });

  it('flags tree_degraded for an error status too', () => {
    const h = deriveSourcePipelineHealth(
      makeStatus({ chunks_pending: 0 }),
      makePipeline({ status: 'error' })
    );
    expect(h.issues).toContain('tree_degraded');
  });

  // -- Multiple layers at once (the full-failure scenario) -------------------
  it('surfaces every failed layer together, embeddings first', () => {
    const h = deriveSourcePipelineHealth(
      makeStatus({ chunks_synced: 1, chunks_pending: 1 }),
      makePipeline({ status: 'degraded', degraded: { semantic_recall: true, structure: true } })
    );
    expect(h.state).toBe('ingested_only');
    // Embeddings + extraction; the generic tree layer is suppressed because
    // more specific layers already explain the degradation.
    expect(h.issues).toEqual(['stored_without_vectors', 'extraction_failed']);
  });
});

// -- Layer 1, the soft half: a backlog that is being drained (openhuman#6025)
describe('deriveSourcePipelineHealth — vectors pending vs stored without vectors', () => {
  it('reads pending chunks as vectors_pending while a re-embed chain has rows to process', () => {
    // The incident's numbers: 676 synced, 322 waiting, the backfill row queued
    // behind extraction. Nothing failed; the row must not say it did.
    const h = deriveSourcePipelineHealth(
      makeStatus({ chunks_synced: 676, chunks_pending: 322, freshness: 'idle' }),
      makePipeline(),
      makeBackfill({ in_progress: true, pending_jobs: 1 })
    );
    expect(h.state).toBe('vectors_pending');
    expect(h.vectorsPending).toBe(true);
    expect(h.issues).toEqual([]);
    expect(h.authRelated).toBe(false);
  });

  it('reads pending chunks as vectors_pending while the newest chunk is active, with no backfill snapshot', () => {
    // The backfill RPC failed (or predates this build): the source's own
    // freshness carries the "still moving" judgement on its own.
    const h = deriveSourcePipelineHealth(
      makeStatus({ chunks_synced: 10, chunks_pending: 4, freshness: 'active' }),
      null
    );
    expect(h.state).toBe('vectors_pending');
  });

  it('reads pending chunks as vectors_pending while the newest chunk is recent, with no backfill snapshot', () => {
    const h = deriveSourcePipelineHealth(
      makeStatus({ chunks_synced: 10, chunks_pending: 4, freshness: 'recent' }),
      null,
      null
    );
    expect(h.state).toBe('vectors_pending');
  });

  it('lets an explicit "no chain queued" snapshot outrank a fresh chunk', () => {
    // `last_chunk_at_ms` is the content's own time (an email's sent date,
    // a file's mtime), not proof that a worker is alive; a future-dated item
    // reads `active` forever. When the engine says no chain is queued, the
    // backlog is stuck whatever the freshness pill shows.
    const h = deriveSourcePipelineHealth(
      makeStatus({ chunks_synced: 10, chunks_pending: 4, freshness: 'active' }),
      makePipeline(),
      makeBackfill({ in_progress: false })
    );
    expect(h.state).toBe('ingested_only');
    expect(h.issues).toEqual(['stored_without_vectors']);
    expect(h.vectorsPending).toBe(false);
  });

  it('flags stored_without_vectors once the backlog is idle with no chain queued', () => {
    const h = deriveSourcePipelineHealth(
      makeStatus({ chunks_synced: 10, chunks_pending: 4, freshness: 'idle' }),
      makePipeline(),
      makeBackfill({ in_progress: false })
    );
    expect(h.state).toBe('ingested_only');
    expect(h.issues).toEqual(['stored_without_vectors']);
    expect(h.vectorsPending).toBe(false);
  });

  it('lets the global recall latch win over a draining backfill', () => {
    const h = deriveSourcePipelineHealth(
      makeStatus({ chunks_pending: 4, freshness: 'active' }),
      makePipeline({ status: 'degraded', degraded: { semantic_recall: true, structure: false } }),
      makeBackfill({ in_progress: true })
    );
    expect(h.state).toBe('ingested_only');
    expect(h.issues).toContain('stored_without_vectors');
    expect(h.vectorsPending).toBe(false);
  });

  it.each([
    'budget_exhausted',
    'auth_missing',
    'auth_invalid',
    'embeddings_unconfigured',
    'embedding_dim_mismatch',
    'local_model_unavailable',
  ] as const)(
    'lets an embeddings-family blocking cause (%s) win over a draining backfill',
    code => {
      // The provider cannot write vectors, so the backlog is not going to drain
      // whatever the chain flag says; that is the hard state, sign-in CTA and all.
      const h = deriveSourcePipelineHealth(
        makeStatus({ chunks_pending: 4, freshness: 'active' }),
        makePipeline({
          status: 'error',
          first_blocking_cause: {
            code,
            class: 'unrecoverable',
            remediation_key: `memory.health.remediation.${code}`,
          },
        }),
        makeBackfill({ in_progress: true })
      );
      expect(h.issues).toContain('stored_without_vectors');
      expect(h.vectorsPending).toBe(false);
      expect(h.authRelated).toBe(code === 'auth_missing');
    }
  );

  it('keeps a non-embeddings blocking cause from turning a draining backlog amber', () => {
    // Extraction timing out says nothing about the embedder; the vectors are
    // still coming. Both truths are carried: the extraction warning AND the
    // neutral pending note.
    const h = deriveSourcePipelineHealth(
      makeStatus({ chunks_pending: 4, freshness: 'active' }),
      makePipeline({
        status: 'degraded',
        first_blocking_cause: {
          code: 'extraction_timeout',
          class: 'transient',
          remediation_key: 'memory.health.remediation.extraction_timeout',
        },
      })
    );
    expect(h.state).toBe('ingested_only');
    expect(h.issues).toEqual(['extraction_failed']);
    expect(h.vectorsPending).toBe(true);
  });

  it.each([
    { label: 'is_paused', pipeline: makePipeline({ is_paused: true }) },
    { label: "status 'paused'", pipeline: makePipeline({ status: 'paused', reason: 'gate off' }) },
    {
      label: 'gate_paused (live policy, mode still auto)',
      pipeline: makePipeline({ gate_paused: true, gate_pause_reason: 'on_battery' }),
    },
  ])(
    'reads a paused scheduler ($label) as stuck even while a chain is armed and ingest is fresh',
    ({ pipeline }) => {
      // Pausing the memory-tree scheduler stops every LLM-bound job, the embed
      // chain included, while leaving its `in_progress` flag armed and letting
      // ingest keep writing. Nothing will drain the backlog until the user
      // resumes, so "shortly" would be a promise nobody is keeping.
      const h = deriveSourcePipelineHealth(
        makeStatus({ chunks_synced: 10, chunks_pending: 4, freshness: 'active' }),
        pipeline,
        makeBackfill({ in_progress: true, pending_jobs: 1 })
      );
      expect(h.state).toBe('ingested_only');
      expect(h.issues).toEqual(['stored_without_vectors']);
      expect(h.vectorsPending).toBe(false);
    }
  );

  it('reads a stalled queue as stuck even while the backfill snapshot says in progress', () => {
    // The #5324 verdict: eligible work waiting six hours with nothing
    // settling. A stalled `reembed_backfill` row still keeps `in_progress`
    // up, but the core has ruled that nothing is draining, so the row must
    // not promise "shortly" beside a degraded status.
    const h = deriveSourcePipelineHealth(
      makeStatus({ chunks_synced: 10, chunks_pending: 4, freshness: 'active' }),
      makePipeline({ status: 'degraded', reason: 'queue stalled for 7h', queue_stalled: true }),
      makeBackfill({ in_progress: true, pending_jobs: 1 })
    );
    expect(h.state).toBe('ingested_only');
    expect(h.issues).toEqual(['stored_without_vectors']);
    expect(h.vectorsPending).toBe(false);
  });

  it('stays retrieval_ready when a backfill runs but this source has nothing pending', () => {
    const h = deriveSourcePipelineHealth(
      makeStatus({ chunks_pending: 0 }),
      makePipeline(),
      makeBackfill({ in_progress: true, pending_jobs: 1 })
    );
    expect(h.state).toBe('retrieval_ready');
    expect(h.vectorsPending).toBe(false);
  });
});

describe('pipelineIssueMessageKey', () => {
  it('maps every issue kind to a stable i18n key', () => {
    const kinds: SourcePipelineIssueKind[] = [
      'stored_without_vectors',
      'extraction_failed',
      'tree_degraded',
    ];
    for (const k of kinds) {
      expect(pipelineIssueMessageKey(k)).toMatch(/^sync\.pipeline\./);
    }
  });
});
