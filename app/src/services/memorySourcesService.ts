/**
 * RPC client for the memory_sources domain.
 *
 * Wraps `openhuman.memory_sources_*` RPCs so UI components get typed
 * responses without knowing the wire shape.
 */
import debug from 'debug';

import { callCoreRpc } from './coreRpcClient';

const log = debug('memory-sources');

export type SourceKind =
  | 'composio'
  | 'conversation'
  | 'folder'
  | 'github_repo'
  | 'twitter_query'
  | 'rss_feed'
  | 'web_page';

export interface MemorySourceEntry {
  id: string;
  kind: SourceKind;
  label: string;
  enabled: boolean;
  toolkit?: string;
  connection_id?: string;
  path?: string;
  glob?: string;
  url?: string;
  branch?: string;
  paths?: string[];
  query?: string;
  selector?: string;
  // Sync limit fields (all optional; omit = use backend default / unlimited)
  since_days?: number;
  max_items?: number;
  max_commits?: number;
  max_issues?: number;
  max_prs?: number;
  sync_depth_days?: number;
  max_tokens_per_sync?: number;
  max_cost_per_sync_usd?: number;
}

export interface SourceItem {
  id: string;
  title: string;
  updated_at_ms?: number | null;
}

export interface SourceContent {
  id: string;
  title: string;
  body: string;
  content_type: 'markdown' | 'html' | 'plaintext';
  metadata: Record<string, unknown>;
}

function unwrap<T>(raw: unknown): T {
  const obj = raw as Record<string, unknown>;
  if (obj && typeof obj === 'object' && 'result' in obj) {
    return obj.result as T;
  }
  return raw as T;
}

export async function listMemorySources(): Promise<MemorySourceEntry[]> {
  log('list');
  const resp = await callCoreRpc<{ sources: MemorySourceEntry[] }>({
    method: 'openhuman.memory_sources_list',
  });
  const data = unwrap<{ sources: MemorySourceEntry[] }>(resp);
  return data.sources ?? [];
}

export async function getMemorySource(id: string): Promise<MemorySourceEntry | null> {
  log('get id=%s', id);
  const resp = await callCoreRpc<{ source: MemorySourceEntry | null }>({
    method: 'openhuman.memory_sources_get',
    params: { id },
  });
  const data = unwrap<{ source: MemorySourceEntry | null }>(resp);
  return data.source ?? null;
}

export async function addMemorySource(
  params: Omit<MemorySourceEntry, 'id'>
): Promise<MemorySourceEntry> {
  log('add kind=%s label=%s', params.kind, params.label);
  const resp = await callCoreRpc<{ source: MemorySourceEntry }>({
    method: 'openhuman.memory_sources_add',
    params,
  });
  const data = unwrap<{ source: MemorySourceEntry }>(resp);
  return data.source;
}

export async function updateMemorySource(
  id: string,
  patch: Partial<Omit<MemorySourceEntry, 'id' | 'kind'>>
): Promise<MemorySourceEntry> {
  log('update id=%s', id);
  const resp = await callCoreRpc<{ source: MemorySourceEntry }>({
    method: 'openhuman.memory_sources_update',
    params: { id, ...patch },
  });
  const data = unwrap<{ source: MemorySourceEntry }>(resp);
  return data.source;
}

export async function removeMemorySource(id: string): Promise<boolean> {
  log('remove id=%s', id);
  const resp = await callCoreRpc<{ removed: boolean }>({
    method: 'openhuman.memory_sources_remove',
    params: { id },
  });
  const data = unwrap<{ removed: boolean }>(resp);
  return data.removed;
}

export async function listSourceItems(sourceId: string): Promise<SourceItem[]> {
  log('list_items source_id=%s', sourceId);
  const resp = await callCoreRpc<{ items: SourceItem[] }>({
    method: 'openhuman.memory_sources_list_items',
    params: { source_id: sourceId },
  });
  const data = unwrap<{ items: SourceItem[] }>(resp);
  return data.items ?? [];
}

export async function readSourceItem(sourceId: string, itemId: string): Promise<SourceContent> {
  log('read_item source_id=%s item_id=%s', sourceId, itemId);
  const resp = await callCoreRpc<{ content: SourceContent }>({
    method: 'openhuman.memory_sources_read_item',
    params: { source_id: sourceId, item_id: itemId },
  });
  const data = unwrap<{ content: SourceContent }>(resp);
  return data.content;
}

export type FreshnessLabel = 'active' | 'recent' | 'idle';

export interface SourceStatus {
  source_id: string;
  chunks_synced: number;
  chunks_pending: number;
  last_chunk_at_ms: number | null;
  freshness: FreshnessLabel;
  /**
   * The stage of a sync the core has in flight for this source, `null` when
   * none (openhuman#6019). Absent altogether from a core that predates the
   * field — callers treat absent as "unknown", not as idle.
   */
  sync_stage?: string | null;
  /** The detail that came with `sync_stage`, if any. */
  sync_detail?: string | null;
}

export async function memorySourcesStatusList(): Promise<SourceStatus[]> {
  log('status_list');
  const resp = await callCoreRpc<{ statuses: SourceStatus[] }>({
    method: 'openhuman.memory_sources_status_list',
  });
  const data = unwrap<{ statuses: SourceStatus[] }>(resp);
  return data.statuses ?? [];
}

/**
 * Per-call budget for the RPCs that run a whole source sync before answering
 * (`memory_sources_sync`, `memory_sources_apply_all_in`).
 *
 * The client's default per-call timeout is 30 s and one Gmail page alone takes
 * ~31 s end to end, so the default aborted the fetch while the core kept
 * syncing: the row flipped to "failed" for work that then completed, and the
 * run never showed as done (#5820). This is the client's own clamp ceiling
 * (`PER_CALL_TIMEOUT_MAX_MS`); the core's bus deadline is sized to outlast it
 * so this abort, with its clean message, is the one that fires if a run wedges.
 */
export const MEMORY_SYNC_RPC_TIMEOUT_MS = 10 * 60 * 1_000;

export async function syncMemorySource(sourceId: string): Promise<void> {
  log('sync source_id=%s', sourceId);
  await callCoreRpc<{ requested: boolean }>({
    method: 'openhuman.memory_sources_sync',
    params: { source_id: sourceId },
    timeoutMs: MEMORY_SYNC_RPC_TIMEOUT_MS,
  });
}

/**
 * Toolkit slugs that ship a native memory-sync provider (backend registry —
 * `all_providers()`). The Add Source connection picker uses this to disable
 * connections whose toolkit can never sync. Maps to
 * `openhuman.memory_sources_supported_toolkits`. See issue #3352.
 */
export async function getSupportedToolkits(): Promise<string[]> {
  log('supported_toolkits');
  const resp = await callCoreRpc<{ toolkits: string[] }>({
    method: 'openhuman.memory_sources_supported_toolkits',
  });
  const data = unwrap<{ toolkits: string[] }>(resp);
  return data.toolkits ?? [];
}

export interface ApplyAllInResult {
  sources: MemorySourceEntry[];
  sync_triggered: number;
  /**
   * Enabled sources whose sync trigger failed (openhuman#5820). Older cores
   * omit it; the service normalises absence to `0`, so a caller can always
   * read it. `sync_triggered === 0 && sync_failed > 0` is the "nothing
   * started" outcome the toast must not report as success.
   */
  sync_failed: number;
  /** One `<source_id>: <error>` line per failed trigger. */
  sync_errors: string[];
}

/**
 * Enables every memory source, clears all per-source sync caps, and
 * triggers a background sync for each. Equivalent to the UI "All In"
 * action. Maps to `openhuman.memory_sources_apply_all_in`.
 */
export async function applyAllIn(): Promise<ApplyAllInResult> {
  log('apply_all_in');
  // All In runs every enabled source's sync to completion, one after another,
  // before it answers; the budget has to cover the whole sweep.
  const resp = await callCoreRpc<ApplyAllInResult>({
    method: 'openhuman.memory_sources_apply_all_in',
    timeoutMs: MEMORY_SYNC_RPC_TIMEOUT_MS,
  });
  const data = unwrap<ApplyAllInResult>(resp);
  return {
    sources: data.sources ?? [],
    sync_triggered: data.sync_triggered ?? 0,
    sync_failed: data.sync_failed ?? 0,
    sync_errors: data.sync_errors ?? [],
  };
}

export interface CodingSessionSourceStatus {
  kind: 'claude_code' | 'codex';
  available: boolean;
  session_files: number;
  evidence_units: number;
  invalid_files: number;
  scan_truncated?: boolean;
}

export interface CodingSessionIngestResult {
  mode: 'backfill' | 'incremental';
  files_seen: number;
  sessions_processed: number;
  sessions_skipped: number;
  sessions_failed: number;
  evidence_units: number;
  observations: number;
  budget_hit: boolean;
  pack_path?: string | null;
}

// A single ingest RPC is bounded so it fits under the core RPC client's hard
// per-call ceiling (`PER_CALL_TIMEOUT_MAX_MS` = 600s in `coreRpcClient.ts`, which
// clamps every override — a larger request timeout is silently unreachable).
// The bound mirrors the core's `ingest_budget` (`memory/sources/rpc.rs`): the
// server sizes the same call at `120s + N*90s` and the client adds a 15s grace
// so the server's structured timeout fires first. Each multi-window session
// drives several sequential LLM calls (~90s/session), so the batch is kept small
// (`120s + 5*90s + 15s = 585s < 600s`) and large histories drain across repeated
// bounded passes via `drainCodingSessions`, driven by the response `budget_hit`
// flag — never by widening a single call past the reachable ceiling.
const CODING_SESSION_BATCH_MAX = 5;
const CODING_SESSION_BASE_TIMEOUT_MS = 120_000;
const CODING_SESSION_PER_SESSION_TIMEOUT_MS = 90_000;
const CODING_SESSION_RPC_GRACE_MS = 15_000;
// Hard safety cap on drain passes so a stuck backlog can never spin forever.
// Sized well above the largest realistic history: at 5 sessions/pass this covers
// ~10k sessions in a single run, so the target ~7,800-file case drains fully
// rather than exiting capped. The `moreRemaining` flag still lets the UI report
// an honest "paused" state if the cap is ever reached.
const CODING_SESSION_MAX_DRAIN_PASSES = 2000;

export async function getCodingSessionStatus(): Promise<CodingSessionSourceStatus[]> {
  log('coding_session_status: entry');
  const resp = await callCoreRpc<{ sources: CodingSessionSourceStatus[] }>({
    method: 'openhuman.memory_sources_coding_session_status',
  });
  const data = unwrap<{ sources: CodingSessionSourceStatus[] }>(resp);
  log('coding_session_status: exit sources=%d', data.sources?.length ?? 0);
  return data.sources ?? [];
}

export async function ingestCodingSessions(
  backfill = false,
  maxSessions = CODING_SESSION_BATCH_MAX
): Promise<CodingSessionIngestResult> {
  const boundedMaxSessions = Number.isFinite(maxSessions)
    ? Math.min(Math.max(Math.trunc(maxSessions), 1), CODING_SESSION_BATCH_MAX)
    : CODING_SESSION_BATCH_MAX;
  const timeoutMs =
    CODING_SESSION_BASE_TIMEOUT_MS +
    boundedMaxSessions * CODING_SESSION_PER_SESSION_TIMEOUT_MS +
    CODING_SESSION_RPC_GRACE_MS;
  log(
    'ingest_coding_sessions: entry backfill=%s max_sessions=%d requested=%d timeout_ms=%d',
    backfill,
    boundedMaxSessions,
    maxSessions,
    timeoutMs
  );
  const resp = await callCoreRpc<CodingSessionIngestResult>({
    method: 'openhuman.memory_sources_ingest_coding_sessions',
    params: { backfill, max_sessions: boundedMaxSessions },
    timeoutMs,
  });
  const data = unwrap<CodingSessionIngestResult>(resp);
  log(
    'ingest_coding_sessions: exit processed=%d failed=%d budget_hit=%s',
    data.sessions_processed,
    data.sessions_failed,
    data.budget_hit
  );
  return data;
}

export interface CodingSessionDrainProgress {
  /** Bounded ingest RPC passes completed so far in this drain. */
  passes: number;
  /** Sessions distilled across every pass in this drain. */
  sessionsProcessed: number;
  /** Sessions retained for retry after a provider failure (latest pass). */
  sessionsFailed: number;
  /** Persona observations distilled across this drain. */
  observations: number;
  /** Best-effort backlog estimate reported by the latest pass. */
  remaining: number;
  /** True while the backlog still reported more work after the latest pass. */
  moreRemaining: boolean;
  /**
   * True when a pass stopped waiting on a deadline rather than failing.
   *
   * The distinction is the whole point: no deadline in this stack cancels the
   * work it bounds. The core's `ingest_budget` and tinybus' call deadline both
   * only release the *caller* — tinybus says so in as many words: *"A timeout
   * does not cancel the remote work."* So the import is still running, its
   * result is simply not going to arrive on this call, and reporting it as a
   * failure is both wrong and harmful: it invites a retry of a 35-60 s job
   * that already succeeded (#5802).
   */
  timedOut: boolean;
}

/**
 * Whether a rejected ingest pass is a deadline the **core itself** reported.
 *
 * Suppressing a timeout is only defensible when the work is genuinely still
 * running, and that needs proof — not merely the absence of a reply. The proof
 * used here is the message prefix: `ingest coding sessions: ` is added by
 * `ingest_coding_sessions_rpc` (`src/openhuman/memory/sources/rpc.rs`) around
 * both of its deadline paths, so a message carrying it can only have been
 * produced *inside* that handler. The core received the request, resolved the
 * binding and started the call. Two shapes qualify:
 *
 * - `ingest coding sessions: timed out after 570s` — the RPC's own
 *   `ingest_budget` ceiling. Note the unit: **seconds**.
 * - ``ingest coding sessions: call to `IngestCodingSessions` timed out after
 *   30000ms`` — the tinybus call deadline, wrapped by the same handler.
 *   **Milliseconds.**
 *
 * Everything else is a failure, and two near-misses are deliberately excluded:
 *
 * - **The renderer's own `AbortController`** (`CoreRpcError` with
 *   `kind === 'timeout'`, message `Core RPC <method> timed out after <n>ms`).
 *   It proves the client stopped waiting and nothing more. A core that is
 *   wedged, overloaded, or not running produces exactly the same signal as one
 *   that is happily distilling sessions, so treating it as "still running"
 *   would show a false success and discourage a retry that is actually needed
 *   — the same false-reporting defect as #5802 with the sign flipped.
 * - **A bare timeout phrase.** A provider failure surfacing as
 *   `request timed out after 30s` would have matched an unprefixed pattern.
 *
 * That asymmetry is the point: a *false negative* here costs a banner that
 * says "failed" when the work continues — annoying, and recoverable, because
 * the next status poll shows what landed. A *false positive* costs a banner
 * that says "still running" over work that stopped, which is unrecoverable
 * without the user noticing on their own. So the predicate is deliberately
 * narrow and keyed on evidence rather than on symptom.
 */
export function isIngestTimeout(cause: unknown): boolean {
  const message = cause instanceof Error ? cause.message : String(cause);
  return /ingest coding sessions:[\s\S]*timed out after \d+\s*(ms|s)\b/i.test(message);
}

export interface CodingSessionDrainOptions {
  /** Called after each pass so the UI can render live progress. */
  onProgress?: (progress: CodingSessionDrainProgress) => void;
  /** Polled before each pass; return true to pause the drain cleanly. */
  shouldStop?: () => boolean;
  /** Per-pass batch bound; defaults to the RPC-timeout-safe maximum. */
  maxSessionsPerPass?: number;
  /** Hard cap on passes as a runaway guard. */
  maxPasses?: number;
}

/**
 * Drain the coding-session backlog across repeated bounded passes until it is
 * empty, the caller stops it, or a pass makes no forward progress.
 *
 * Incremental mode only, by design: each pass skips already-distilled sessions
 * by cursor, so the backlog strictly shrinks and the loop converges. Backfill
 * re-reads every file regardless of cursor, so looping it under the per-call
 * budget would re-process the same oldest slice forever and never drain — and
 * because evidence ids are content-addressed, an incremental drain reproduces
 * the same persona anyway.
 */
export async function drainCodingSessions(
  options: CodingSessionDrainOptions = {}
): Promise<CodingSessionDrainProgress> {
  const { onProgress, shouldStop } = options;
  const maxSessionsPerPass = options.maxSessionsPerPass ?? CODING_SESSION_BATCH_MAX;
  // Normalize the safety cap: a non-finite or non-positive override would defeat
  // the runaway guard, so fall back to the default in that case.
  const maxPasses =
    Number.isFinite(options.maxPasses) && (options.maxPasses as number) > 0
      ? Math.trunc(options.maxPasses as number)
      : CODING_SESSION_MAX_DRAIN_PASSES;

  const progress: CodingSessionDrainProgress = {
    passes: 0,
    sessionsProcessed: 0,
    sessionsFailed: 0,
    observations: 0,
    remaining: 0,
    moreRemaining: false,
    timedOut: false,
  };
  log('drain_coding_sessions: entry max_per_pass=%d max_passes=%d', maxSessionsPerPass, maxPasses);

  while (progress.passes < maxPasses) {
    if (shouldStop?.()) {
      log('drain_coding_sessions: stop requested after pass=%d', progress.passes);
      break;
    }
    let result: CodingSessionIngestResult;
    try {
      result = await ingestCodingSessions(false, maxSessionsPerPass);
    } catch (cause) {
      // A deadline is not a failure. Nothing in this stack cancels the import
      // when a caller stops waiting for it, so the pass is still running and
      // will still write its observations — it just has no report to hand back
      // on this call. Rethrowing here is what put a red "ingestion failed"
      // banner over a run that went on to succeed, and invited the user to
      // redo 35-60 s of work (#5802).
      //
      // Returning rather than rethrowing keeps every completed pass's counts,
      // which is the honest answer: those passes really did import what they
      // report, and only the last one is unresolved.
      if (!isIngestTimeout(cause)) throw cause;
      progress.timedOut = true;
      log('drain_coding_sessions: pass=%d timed out; work continues', progress.passes + 1);
      onProgress?.({ ...progress });
      break;
    }
    progress.passes += 1;
    progress.sessionsProcessed += result.sessions_processed;
    progress.sessionsFailed = result.sessions_failed;
    progress.observations += result.observations;
    // files_seen is the discovered total for this scan; skipped + processed is
    // what this pass accounted for, so the remainder is the honest backlog.
    progress.remaining = Math.max(
      0,
      result.files_seen - result.sessions_skipped - result.sessions_processed
    );
    progress.moreRemaining = result.budget_hit;
    onProgress?.({ ...progress });

    if (!result.budget_hit) {
      log(
        'drain_coding_sessions: drained after pass=%d processed=%d',
        progress.passes,
        progress.sessionsProcessed
      );
      break;
    }
    if (result.sessions_processed === 0) {
      // The backlog still reports more work, but this pass distilled nothing
      // new — every remaining candidate failed or could not advance. Stop
      // rather than spin; the caller surfaces the retained failures.
      log('drain_coding_sessions: no forward progress on pass=%d — stopping', progress.passes);
      break;
    }
  }

  log(
    'drain_coding_sessions: exit passes=%d processed=%d failed=%d remaining=%d more=%s',
    progress.passes,
    progress.sessionsProcessed,
    progress.sessionsFailed,
    progress.remaining,
    progress.moreRemaining
  );
  return progress;
}

/// i18n keys for each source kind's user-visible label. Resolve via
/// `t(SOURCE_KIND_LABEL_KEYS[kind])` in components — keeping the keys
/// as a constant lets the dialog kind-picker render the same labels
/// without each call site duplicating the switch.
export const SOURCE_KIND_LABEL_KEYS: Record<SourceKind, string> = {
  composio: 'memorySources.kind.composio',
  conversation: 'memorySources.kind.conversation',
  folder: 'memorySources.kind.folder',
  github_repo: 'memorySources.kind.github_repo',
  twitter_query: 'memorySources.kind.twitter_query',
  rss_feed: 'memorySources.kind.rss_feed',
  web_page: 'memorySources.kind.web_page',
};

export const SOURCE_KIND_ICONS: Record<SourceKind, string> = {
  composio: '🔗',
  conversation: '💬',
  folder: '📁',
  github_repo: '🐙',
  twitter_query: '🐦',
  rss_feed: '📡',
  web_page: '🌐',
};
