import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from './coreRpcClient';
import {
  addMemorySource,
  applyAllIn,
  type CodingSessionDrainProgress,
  drainCodingSessions,
  getCodingSessionStatus,
  ingestCodingSessions,
  isIngestTimeout,
  listMemorySources,
  MEMORY_SYNC_RPC_TIMEOUT_MS,
  type MemorySourceEntry,
  removeMemorySource,
  SOURCE_KIND_ICONS,
  SOURCE_KIND_LABEL_KEYS,
  updateMemorySource,
} from './memorySourcesService';

vi.mock('./coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const mockedCall = vi.mocked(callCoreRpc);

describe('memorySourcesService', () => {
  beforeEach(() => {
    mockedCall.mockReset();
  });

  it('listMemorySources returns sources from envelope-wrapped response', async () => {
    mockedCall.mockResolvedValue({
      result: {
        sources: [{ id: 'src_1', kind: 'folder', label: 'Notes', enabled: true, path: '/tmp' }],
      },
      logs: [],
    } as never);

    const sources = await listMemorySources();

    expect(mockedCall).toHaveBeenCalledWith({ method: 'openhuman.memory_sources_list' });
    expect(sources).toHaveLength(1);
    expect(sources[0].kind).toBe('folder');
  });

  it('listMemorySources handles flat (un-wrapped) response', async () => {
    mockedCall.mockResolvedValue({ sources: [] } as never);
    const sources = await listMemorySources();
    expect(sources).toEqual([]);
  });

  it('addMemorySource sends kind-specific flat fields', async () => {
    mockedCall.mockResolvedValue({
      result: {
        source: { id: 'src_new', kind: 'folder', label: 'Test', enabled: true, path: '/x' },
      },
      logs: [],
    } as never);

    const result = await addMemorySource({
      kind: 'folder',
      label: 'Test',
      enabled: true,
      path: '/x',
    });

    expect(mockedCall).toHaveBeenCalledWith({
      method: 'openhuman.memory_sources_add',
      params: { kind: 'folder', label: 'Test', enabled: true, path: '/x' },
    });
    expect(result.id).toBe('src_new');
  });

  it('updateMemorySource sends id + patch fields', async () => {
    mockedCall.mockResolvedValue({
      result: { source: { id: 'src_1', kind: 'folder', label: 'X', enabled: false } },
      logs: [],
    } as never);

    await updateMemorySource('src_1', { enabled: false, label: 'X' });

    expect(mockedCall).toHaveBeenCalledWith({
      method: 'openhuman.memory_sources_update',
      params: { id: 'src_1', enabled: false, label: 'X' },
    });
  });

  it('removeMemorySource returns boolean', async () => {
    mockedCall.mockResolvedValue({ result: { removed: true }, logs: [] } as never);
    const removed = await removeMemorySource('src_1');
    expect(removed).toBe(true);
  });

  it('exposes labels and icons for every source kind', () => {
    const kinds = [
      'composio',
      'conversation',
      'folder',
      'github_repo',
      'twitter_query',
      'rss_feed',
      'web_page',
    ] as const;
    for (const kind of kinds) {
      expect(SOURCE_KIND_LABEL_KEYS[kind]).toBeTruthy();
      expect(SOURCE_KIND_ICONS[kind]).toBeTruthy();
    }
  });

  it('addMemorySource sends conversation source with no extra fields', async () => {
    mockedCall.mockResolvedValue({
      result: {
        source: { id: 'src_conv', kind: 'conversation', label: 'Conversations', enabled: true },
      },
      logs: [],
    } as never);

    const result = await addMemorySource({
      kind: 'conversation',
      label: 'Conversations',
      enabled: true,
    });

    expect(mockedCall).toHaveBeenCalledWith({
      method: 'openhuman.memory_sources_add',
      params: { kind: 'conversation', label: 'Conversations', enabled: true },
    });
    expect(result.id).toBe('src_conv');
    expect(result.kind).toBe('conversation');
  });

  // ---------------------------------------------------------------------------
  // applyAllIn
  // ---------------------------------------------------------------------------

  it('applyAllIn calls the correct RPC method', async () => {
    mockedCall.mockResolvedValue({
      result: {
        sources: [{ id: 'src_1', kind: 'github_repo', label: 'All Repos', enabled: true }],
        sync_triggered: 1,
      },
      logs: [],
    } as never);

    const result = await applyAllIn();

    expect(mockedCall).toHaveBeenCalledWith({
      method: 'openhuman.memory_sources_apply_all_in',
      timeoutMs: MEMORY_SYNC_RPC_TIMEOUT_MS,
    });
    expect(result.sync_triggered).toBe(1);
    expect(result.sources).toHaveLength(1);
    expect(result.sources[0].id).toBe('src_1');
  });

  it('applyAllIn normalises the #5820 failure fields for cores that omit them', async () => {
    mockedCall.mockResolvedValueOnce({
      result: { sources: [], sync_triggered: 0 },
      logs: [],
    } as never);
    const legacy = await applyAllIn();
    expect(legacy.sync_failed).toBe(0);
    expect(legacy.sync_errors).toEqual([]);

    mockedCall.mockResolvedValueOnce({
      result: { sources: [], sync_triggered: 0, sync_failed: 2, sync_errors: ['a: x', 'b: y'] },
      logs: [],
    } as never);
    const failed = await applyAllIn();
    expect(failed.sync_failed).toBe(2);
    expect(failed.sync_errors).toEqual(['a: x', 'b: y']);
  });

  it('applyAllIn handles envelope-wrapped response', async () => {
    mockedCall.mockResolvedValue({ result: { sources: [], sync_triggered: 0 }, logs: [] } as never);

    const result = await applyAllIn();
    expect(result.sources).toEqual([]);
    expect(result.sync_triggered).toBe(0);
  });

  it('applyAllIn handles flat (un-wrapped) response', async () => {
    mockedCall.mockResolvedValue({
      sources: [{ id: 'src_2', kind: 'folder', label: 'Docs', enabled: true }],
      sync_triggered: 1,
    } as never);

    const result = await applyAllIn();
    expect(result.sources).toHaveLength(1);
  });

  // ---------------------------------------------------------------------------
  // MemorySourceEntry interface includes all limit fields
  // ---------------------------------------------------------------------------

  it('MemorySourceEntry interface accepts all limit fields at compile-time', () => {
    // This test is a type-level assertion: if the fields are missing from the
    // interface the assignment below would fail TypeScript compilation.
    const entry: MemorySourceEntry = {
      id: 'src_1',
      kind: 'github_repo',
      label: 'Test',
      enabled: true,
      max_commits: 100,
      max_issues: 200,
      max_prs: 50,
      max_items: 500,
      since_days: 30,
      sync_depth_days: 90,
      max_tokens_per_sync: 100_000,
      max_cost_per_sync_usd: 1.5,
    };
    expect(entry.max_commits).toBe(100);
    expect(entry.max_issues).toBe(200);
    expect(entry.max_prs).toBe(50);
    expect(entry.since_days).toBe(30);
    expect(entry.sync_depth_days).toBe(90);
    expect(entry.max_tokens_per_sync).toBe(100_000);
    expect(entry.max_cost_per_sync_usd).toBe(1.5);
  });

  it('discovers Codex and Claude Code session sources', async () => {
    mockedCall.mockResolvedValue({
      result: {
        sources: [
          { kind: 'codex', available: true, session_files: 2, evidence_units: 5, invalid_files: 0 },
        ],
      },
      logs: [],
    } as never);

    const sources = await getCodingSessionStatus();

    expect(mockedCall).toHaveBeenCalledWith({
      method: 'openhuman.memory_sources_coding_session_status',
    });
    expect(sources[0]).toMatchObject({ kind: 'codex', evidence_units: 5 });
  });

  it('requests a timeout-aligned incremental coding-session batch', async () => {
    mockedCall.mockResolvedValue({
      result: {
        mode: 'incremental',
        files_seen: 2,
        sessions_processed: 2,
        sessions_skipped: 0,
        sessions_failed: 0,
        evidence_units: 5,
        observations: 3,
        budget_hit: false,
      },
      logs: [],
    } as never);

    const result = await ingestCodingSessions(false, 25);

    // Clamped to the batch max (5), and the per-call timeout stays under the RPC
    // client's hard 600s ceiling: 120s + 5*90s + 15s = 585s.
    expect(mockedCall).toHaveBeenCalledWith({
      method: 'openhuman.memory_sources_ingest_coding_sessions',
      params: { backfill: false, max_sessions: 5 },
      timeoutMs: 585_000,
    });
    // Guard the cross-wire contract: the per-call timeout must stay under
    // coreRpcClient's PER_CALL_TIMEOUT_MAX_MS (600s), or the override is clamped
    // and silently unreachable (the #5509 gap). Raising BATCH_MAX / per-session
    // past this must fail here.
    const CORE_RPC_PER_CALL_TIMEOUT_MAX_MS = 10 * 60 * 1_000;
    expect(585_000).toBeLessThanOrEqual(CORE_RPC_PER_CALL_TIMEOUT_MAX_MS);
    expect(result.sessions_processed).toBe(2);
  });

  const ingestResult = (over: Partial<Record<string, number | boolean>> = {}) =>
    ({
      result: {
        mode: 'incremental',
        files_seen: 40,
        sessions_processed: 15,
        sessions_skipped: 0,
        sessions_failed: 0,
        evidence_units: 30,
        observations: 20,
        budget_hit: true,
        ...over,
      },
      logs: [],
    }) as never;

  it('drains across bounded passes until the backlog reports no more budget', async () => {
    // Two budget-hit passes, then a final pass that clears the backlog.
    mockedCall
      .mockResolvedValueOnce(ingestResult({ sessions_skipped: 0, budget_hit: true }))
      .mockResolvedValueOnce(ingestResult({ sessions_skipped: 15, budget_hit: true }))
      .mockResolvedValueOnce(
        ingestResult({ sessions_processed: 10, sessions_skipped: 30, budget_hit: false })
      );

    const seen: CodingSessionDrainProgress[] = [];
    const result = await drainCodingSessions({ onProgress: p => seen.push(p) });

    expect(mockedCall).toHaveBeenCalledTimes(3);
    // Every pass stays bounded to the timeout-safe per-call maximum.
    expect(mockedCall).toHaveBeenLastCalledWith(
      expect.objectContaining({ params: { backfill: false, max_sessions: 5 } })
    );
    expect(result.passes).toBe(3);
    expect(result.sessionsProcessed).toBe(40); // 15 + 15 + 10
    expect(result.observations).toBe(60); // 20 * 3
    expect(result.moreRemaining).toBe(false);
    expect(seen).toHaveLength(3);
  });

  it('stops before a pass when shouldStop is asserted', async () => {
    mockedCall.mockResolvedValue(ingestResult({ budget_hit: true }));
    let calls = 0;

    const result = await drainCodingSessions({
      // Allow one pass, then request a stop before the next.
      shouldStop: () => calls++ >= 1,
    });

    expect(mockedCall).toHaveBeenCalledTimes(1);
    expect(result.passes).toBe(1);
    expect(result.moreRemaining).toBe(true);
  });

  it('reports a deadline as still-running, not as a thrown failure', async () => {
    // The #5802 shape: a pass completes, the next one trips a deadline. No
    // deadline in this stack cancels the import, so the run is still going —
    // rejecting here is what put a red "failed" banner over work that
    // succeeded seconds later.
    mockedCall
      .mockResolvedValueOnce(ingestResult({ sessions_processed: 3, budget_hit: true }))
      .mockRejectedValueOnce(
        new Error('ingest coding sessions: call to `IngestCodingSessions` timed out after 30000ms')
      );

    const result = await drainCodingSessions();

    expect(result.timedOut).toBe(true);
    // The completed pass's work is kept: those sessions really were imported.
    expect(result.sessionsProcessed).toBe(3);
    expect(result.passes).toBe(1);
  });

  it('reports the core budget deadline too, which is spelled in seconds', async () => {
    // `ingest_budget` renders "timed out after 570s". The seconds form does not
    // match coreRpcClient's `\d+ms` timeout classifier, so a check that keyed
    // only on CoreRpcError.kind would let this one through as a failure.
    mockedCall.mockRejectedValueOnce(new Error('ingest coding sessions: timed out after 570s'));

    const result = await drainCodingSessions();

    expect(result.timedOut).toBe(true);
    expect(result.sessionsProcessed).toBe(0);
  });

  it('still rejects a genuine ingest failure', async () => {
    // The other half of the contract: making a deadline non-fatal must not make
    // everything non-fatal.
    mockedCall.mockRejectedValueOnce(new Error('persona pipeline failed'));

    await expect(drainCodingSessions()).rejects.toThrow('persona pipeline failed');
  });

  it('rejects a renderer-side abort instead of calling it still-running', async () => {
    // The client's own AbortController proves the renderer stopped waiting and
    // nothing else — a wedged, overloaded or absent core produces exactly the
    // same signal as one happily distilling sessions. Suppressing it would show
    // a false success over work that may never have started, which is #5802
    // with the sign flipped. Only the core's own prefix is evidence.
    const abort = Object.assign(
      new Error(
        'Core RPC openhuman.memory_sources_ingest_coding_sessions timed out after 585000ms'
      ),
      { kind: 'timeout' }
    );
    mockedCall.mockRejectedValueOnce(abort);

    await expect(drainCodingSessions()).rejects.toThrow('timed out after 585000ms');
  });

  it('isIngestTimeout suppresses only deadlines the core reported', () => {
    // Suppressed: both shapes carry `ingest coding sessions: `, which only
    // `ingest_coding_sessions_rpc` adds — proof the handler ran.
    expect(
      isIngestTimeout(
        new Error('ingest coding sessions: call to `IngestCodingSessions` timed out after 30000ms')
      )
    ).toBe(true);
    expect(isIngestTimeout(new Error('ingest coding sessions: timed out after 570s'))).toBe(true);

    // Not suppressed. A bare timeout phrase carries no evidence that anything
    // was dispatched, and an unprefixed pattern would have swallowed an
    // ordinary provider failure.
    expect(isIngestTimeout(new Error('timed out after 30000ms'))).toBe(false);
    expect(isIngestTimeout(new Error('request timed out after 30s'))).toBe(false);
    expect(isIngestTimeout(new Error('Core RPC x timed out after 585000ms'))).toBe(false);
    expect(isIngestTimeout(new Error('the request timed out'))).toBe(false);
    expect(isIngestTimeout(new Error('session scan failed'))).toBe(false);
    // A kind field alone is not evidence either.
    expect(isIngestTimeout({ kind: 'timeout' })).toBe(false);
  });

  it('stops when a budget-hit pass makes no forward progress', async () => {
    // Backlog still reports budget_hit, but nothing new is distilled — avoid an
    // infinite loop on sessions that only ever fail.
    mockedCall.mockResolvedValue(
      ingestResult({ sessions_processed: 0, sessions_failed: 3, budget_hit: true })
    );

    const result = await drainCodingSessions();

    expect(mockedCall).toHaveBeenCalledTimes(1);
    expect(result.sessionsFailed).toBe(3);
    expect(result.moreRemaining).toBe(true);
  });
});
