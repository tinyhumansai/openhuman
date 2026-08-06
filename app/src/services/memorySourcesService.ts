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
}

export async function memorySourcesStatusList(): Promise<SourceStatus[]> {
  log('status_list');
  const resp = await callCoreRpc<{ statuses: SourceStatus[] }>({
    method: 'openhuman.memory_sources_status_list',
  });
  const data = unwrap<{ statuses: SourceStatus[] }>(resp);
  return data.statuses ?? [];
}

export async function syncMemorySource(sourceId: string): Promise<void> {
  log('sync source_id=%s', sourceId);
  await callCoreRpc<{ requested: boolean }>({
    method: 'openhuman.memory_sources_sync',
    params: { source_id: sourceId },
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
}

/**
 * Enables every memory source, clears all per-source sync caps, and
 * triggers a background sync for each. Equivalent to the UI "All In"
 * action. Maps to `openhuman.memory_sources_apply_all_in`.
 */
export async function applyAllIn(): Promise<ApplyAllInResult> {
  log('apply_all_in');
  const resp = await callCoreRpc<ApplyAllInResult>({
    method: 'openhuman.memory_sources_apply_all_in',
  });
  const data = unwrap<ApplyAllInResult>(resp);
  return { sources: data.sources ?? [], sync_triggered: data.sync_triggered ?? 0 };
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

// A single ingest RPC is bounded so it stays under the core RPC client's
// ten-minute ceiling: 120s + 15 * 30s + 15s ≈ 585s. This is a per-call bound,
// not a per-run one — large histories drain across repeated bounded passes via
// `drainCodingSessions`, driven by the response `budget_hit` flag. Raising this
// to the backend's 1,000-session max would blow the RPC timeout on the first
// call, so the cap stays and the loop does the scaling.
const CODING_SESSION_BATCH_MAX = 15;
const CODING_SESSION_BASE_TIMEOUT_MS = 120_000;
const CODING_SESSION_PER_SESSION_TIMEOUT_MS = 30_000;
const CODING_SESSION_RPC_GRACE_MS = 15_000;
// Hard safety cap on drain passes so a stuck backlog can never spin forever.
// Sized well above the largest realistic history: at 15 sessions/pass this
// covers ~30k sessions in a single run, so the target ~7,800-file case drains
// fully rather than exiting capped. The `moreRemaining` flag still lets the UI
// report an honest "paused" state if the cap is ever reached.
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
  };
  log('drain_coding_sessions: entry max_per_pass=%d max_passes=%d', maxSessionsPerPass, maxPasses);

  while (progress.passes < maxPasses) {
    if (shouldStop?.()) {
      log('drain_coding_sessions: stop requested after pass=%d', progress.passes);
      break;
    }
    const result = await ingestCodingSessions(false, maxSessionsPerPass);
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
