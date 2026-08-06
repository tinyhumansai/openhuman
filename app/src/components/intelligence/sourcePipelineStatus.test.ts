/**
 * Unit tests for the layered Data Sync pipeline-health derivation (GH-4690).
 * Covers each warning layer plus the healthy no-regression case.
 */
import { describe, expect, it } from 'vitest';

import type { SourceStatus } from '../../services/memorySourcesService';
import type { MemoryTreePipelineStatus } from '../../utils/tauriCommands/memoryTree';
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
    // The exact issue repro: "1 chunk / 1 pending" with no pipeline snapshot.
    const h = deriveSourcePipelineHealth(makeStatus({ chunks_synced: 1, chunks_pending: 1 }), null);
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
