/**
 * memory_tree subsystem commands.
 *
 * Thin wrappers over the `openhuman.memory_tree_*` JSON-RPC surface that
 * powers the Memory tab and the Settings → AI backend chooser. Method
 * shapes mirror the Rust handlers in `src/openhuman/memory/tree/read_rpc.rs`
 * and `schemas.rs`.
 *
 * Responses come back wrapped by `RpcOutcome::single_log` as
 * `{ result: <T>, logs: string[] }` (single-log envelope). Each helper
 * unwraps `result` so callers see the bare value the Rust handler
 * returned, falling back gracefully if a future handler stops emitting
 * logs and the bare value flows through.
 *
 * Logging convention: `[memory-tree-rpc]` prefix for grep-friendly tracing
 * per the project debug-logging rule.
 */
import { callCoreRpc } from '../../services/coreRpcClient';

// ── Public types — match the memory_tree RPC contract ────────────────────

/**
 * Source kind values the Rust core uses for canonical chunk metadata.
 * The list is closed for the surfaces the Memory tab cares about, but
 * the wire type is `string` so any future kind round-trips through the
 * UI without a recompile.
 */
export type SourceKind = 'email' | 'chat' | 'screen' | 'voice' | 'doc';

/** Chunk lifecycle phase as emitted by the admission gate. */
export type LifecycleStatus = 'admitted' | 'buffered' | 'pending_extraction' | 'dropped';

/**
 * Canonical entity-kind strings emitted by the entity index. Kept
 * permissive (`string`) on the Rust side; the TS union is the curated
 * subset the UI knows how to render.
 */
export type EntityKind =
  | 'person'
  | 'organization'
  | 'location'
  | 'event'
  | 'product'
  | 'datetime'
  | 'technology'
  | 'artifact'
  | 'quantity'
  | 'misc';

/**
 * A single chunk in the memory tree — one user-visible message-sized unit
 * (an email, a chat turn, a doc page, a transcribed voice clip).
 *
 * Wire shape mirrors Rust's [`ChunkRow`](src/openhuman/memory/tree/read_rpc.rs)
 * — body is replaced with a `≤500-char preview` plus a flag indicating
 * whether the row has an embedding.
 */
export interface Chunk {
  id: string;
  source_kind: SourceKind;
  source_id: string;
  source_ref?: string;
  owner: string;
  timestamp_ms: number;
  token_count: number;
  lifecycle_status: LifecycleStatus;
  content_path?: string;
  /** Up to 500 chars; used as the result-list subject preview. */
  content_preview?: string;
  has_embedding: boolean;
  /** Hierarchical: ["person/Steve-Enamakel", "organization/TinyHumans"]. */
  tags: string[];
}

export interface ChunkFilter {
  source_kinds?: string[];
  source_ids?: string[];
  entity_ids?: string[];
  since_ms?: number;
  until_ms?: number;
  query?: string;
  limit?: number;
  offset?: number;
}

export interface ListChunksResponse {
  chunks: Chunk[];
  total: number;
}

/**
 * Distinct ingest source as returned by `memory_tree_list_sources`.
 *
 * `lifecycle_status` is **optional** — the Rust handler does not emit it
 * (it's a UI-derived aggregate), but the navigator pane wants a per-source
 * dot color. Consumers compute it from chunk-level state and pass it in,
 * or omit it and the UI falls back to a neutral dot.
 */
export interface Source {
  source_id: string;
  /** Un-slugged readable; user-email stripped when `user_email_hint` matched. */
  display_name: string;
  source_kind: string;
  chunk_count: number;
  most_recent_ms: number;
  lifecycle_status?: LifecycleStatus;
}

export interface EntityRef {
  /** Canonical id (e.g. `person:Steven Enamakel`, `email:alice@example.com`). */
  entity_id: string;
  kind: string;
  surface: string;
  count: number;
}

export interface ScoreSignal {
  name: string;
  weight: number;
  value: number;
}

export interface ScoreBreakdown {
  signals: ScoreSignal[];
  total: number;
  threshold: number;
  kept: boolean;
  llm_consulted: boolean;
}

export interface RecallResponse {
  chunks: Chunk[];
  scores: number[];
}

/**
 * Response shape for `memory_tree_delete_chunk`. The Rust handler also
 * surfaces the number of dependent rows removed so UIs can render a
 * detailed "purged X / Y / Z" toast.
 */
export interface DeleteChunkResponse {
  deleted: boolean;
  score_rows_removed: number;
  entity_index_rows_removed: number;
}

/** Backend selector value. */
export type LlmBackend = 'cloud' | 'local';

export interface LlmResponse {
  current: LlmBackend;
}

/**
 * Wire shape for `openhuman.memory_tree_set_llm`.
 *
 * `backend` is required and always overwrites `memory_tree.llm_backend`.
 *
 * The three model fields are optional; absent means "leave the
 * corresponding `memory_tree.*_model` config key untouched", present
 * means "overwrite it". This lets the UI flip the backend without
 * touching models, or persist a per-role model selection without having
 * to re-supply every other model id. Field names are snake_case to match
 * the Rust `SetLlmRequest` struct verbatim — the wrapper does not
 * translate.
 */
export interface SetLlmRequest {
  backend: LlmBackend;
  cloud_model?: string;
  extract_model?: string;
  summariser_model?: string;
}

// ── Envelope unwrap helper ────────────────────────────────────────────────

/**
 * Internal envelope shape produced by `RpcOutcome::single_log` on the
 * Rust side. Every read_rpc handler emits at least one log line, so the
 * shape will be `{ result, logs }` in practice — but we keep the
 * fallback path for defensive parsing.
 */
interface ResultEnvelope<T> {
  result?: T;
  logs?: string[];
}

function unwrapResult<T>(resp: T | ResultEnvelope<T>): T {
  if (resp && typeof resp === 'object' && 'result' in resp) {
    return (resp as ResultEnvelope<T>).result as T;
  }
  return resp as T;
}

// ── memory_tree_list_chunks ──────────────────────────────────────────────

/**
 * Paginated chunk listing with optional filters. Backed by
 * `openhuman.memory_tree_list_chunks`.
 */
export async function memoryTreeListChunks(filter: ChunkFilter): Promise<ListChunksResponse> {
  console.debug('[memory-tree-rpc] memoryTreeListChunks: entry filter=%o', filter);
  const resp = await callCoreRpc<ListChunksResponse | ResultEnvelope<ListChunksResponse>>({
    method: 'openhuman.memory_tree_list_chunks',
    params: filter,
  });
  const out = unwrapResult(resp);
  console.debug(
    '[memory-tree-rpc] memoryTreeListChunks: exit n=%d total=%d',
    out.chunks?.length ?? 0,
    out.total ?? 0
  );
  return out;
}

// ── memory_tree_list_sources ─────────────────────────────────────────────

/**
 * Distinct (source_kind, source_id) pairs with chunk counts and most-recent
 * timestamps. `user_email_hint` (when supplied) tells the Rust handler to
 * strip that address from email-thread display names.
 */
export async function memoryTreeListSources(userEmailHint?: string): Promise<Source[]> {
  console.debug(
    '[memory-tree-rpc] memoryTreeListSources: entry hint=%s',
    userEmailHint ?? '<none>'
  );
  const params = userEmailHint ? { user_email_hint: userEmailHint } : {};
  const resp = await callCoreRpc<Source[] | ResultEnvelope<Source[]>>({
    method: 'openhuman.memory_tree_list_sources',
    params,
  });
  const out = unwrapResult(resp);
  console.debug('[memory-tree-rpc] memoryTreeListSources: exit n=%d', out?.length ?? 0);
  return out ?? [];
}

// ── memory_tree_search ───────────────────────────────────────────────────

/**
 * Keyword `LIKE`-search over chunk bodies. Cheap, deterministic; useful
 * as a fallback when semantic recall is unavailable.
 */
export async function memoryTreeSearch(query: string, k: number): Promise<Chunk[]> {
  console.debug('[memory-tree-rpc] memoryTreeSearch: entry query_len=%d k=%d', query.length, k);
  const resp = await callCoreRpc<Chunk[] | ResultEnvelope<Chunk[]>>({
    method: 'openhuman.memory_tree_search',
    params: { query, k },
  });
  const out = unwrapResult(resp);
  console.debug('[memory-tree-rpc] memoryTreeSearch: exit n=%d', out?.length ?? 0);
  return out ?? [];
}

// ── memory_tree_recall ───────────────────────────────────────────────────

/**
 * Semantic recall via the Phase 4 cosine rerank path. Returns leaf chunks
 * and a parallel `scores` array.
 */
export async function memoryTreeRecall(query: string, k: number): Promise<RecallResponse> {
  console.debug('[memory-tree-rpc] memoryTreeRecall: entry query_len=%d k=%d', query.length, k);
  const resp = await callCoreRpc<RecallResponse | ResultEnvelope<RecallResponse>>({
    method: 'openhuman.memory_tree_recall',
    params: { query, k },
  });
  const out = unwrapResult(resp);
  console.debug('[memory-tree-rpc] memoryTreeRecall: exit n=%d', out?.chunks?.length ?? 0);
  return out ?? { chunks: [], scores: [] };
}

// ── memory_tree_entity_index_for ─────────────────────────────────────────

/**
 * All canonical entities indexed against a single chunk (or summary node) id.
 */
export async function memoryTreeEntityIndexFor(chunkId: string): Promise<EntityRef[]> {
  console.debug('[memory-tree-rpc] memoryTreeEntityIndexFor: entry chunk_id=%s', chunkId);
  const resp = await callCoreRpc<EntityRef[] | ResultEnvelope<EntityRef[]>>({
    method: 'openhuman.memory_tree_entity_index_for',
    params: { chunk_id: chunkId },
  });
  const out = unwrapResult(resp);
  console.debug('[memory-tree-rpc] memoryTreeEntityIndexFor: exit n=%d', out?.length ?? 0);
  return out ?? [];
}

// ── memory_tree_chunks_for_entity ────────────────────────────────────────

/**
 * Inverse of `memoryTreeEntityIndexFor` — return chunk IDs that reference
 * the given entity. Used by the Memory tab's People/Topics lenses to
 * filter the chunk list to those mentioning a selected entity.
 */
export async function memoryTreeChunksForEntity(entityId: string): Promise<string[]> {
  console.debug('[memory-tree-rpc] memoryTreeChunksForEntity: entry entity_id=%s', entityId);
  const resp = await callCoreRpc<string[] | ResultEnvelope<string[]>>({
    method: 'openhuman.memory_tree_chunks_for_entity',
    params: { entity_id: entityId },
  });
  const out = unwrapResult(resp);
  console.debug('[memory-tree-rpc] memoryTreeChunksForEntity: exit n=%d', out?.length ?? 0);
  return out ?? [];
}

// ── memory_tree_top_entities ─────────────────────────────────────────────

/**
 * Most-frequent canonical entities across the workspace, optionally narrowed
 * by `kind`. The Rust handler treats `limit` as required; we default to 50
 * to match the navigator's lens cardinality.
 */
export async function memoryTreeTopEntities(kind?: string, limit = 50): Promise<EntityRef[]> {
  console.debug(
    '[memory-tree-rpc] memoryTreeTopEntities: entry kind=%s limit=%d',
    kind ?? '<all>',
    limit
  );
  const params: Record<string, unknown> = { limit };
  if (kind) params.kind = kind;
  const resp = await callCoreRpc<EntityRef[] | ResultEnvelope<EntityRef[]>>({
    method: 'openhuman.memory_tree_top_entities',
    params,
  });
  const out = unwrapResult(resp);
  console.debug('[memory-tree-rpc] memoryTreeTopEntities: exit n=%d', out?.length ?? 0);
  return out ?? [];
}

// ── memory_tree_chunk_score ──────────────────────────────────────────────

/**
 * Score breakdown stored in `mem_tree_score` for one chunk. Returns
 * `null` when the chunk has no score row (e.g. it was admitted before
 * scoring was enabled, or it is a synthesized fixture in tests).
 */
export async function memoryTreeChunkScore(chunkId: string): Promise<ScoreBreakdown | null> {
  console.debug('[memory-tree-rpc] memoryTreeChunkScore: entry chunk_id=%s', chunkId);
  const resp = await callCoreRpc<ScoreBreakdown | null | ResultEnvelope<ScoreBreakdown | null>>({
    method: 'openhuman.memory_tree_chunk_score',
    params: { chunk_id: chunkId },
  });
  const out = unwrapResult(resp);
  console.debug('[memory-tree-rpc] memoryTreeChunkScore: exit kept=%o', out?.kept);
  return out ?? null;
}

// ── memory_tree_delete_chunk ─────────────────────────────────────────────

/**
 * Purge one chunk plus its score row, entity-index rows, and on-disk .md
 * file. Idempotent — missing chunk returns `deleted=false`.
 */
export async function memoryTreeDeleteChunk(chunkId: string): Promise<DeleteChunkResponse> {
  console.debug('[memory-tree-rpc] memoryTreeDeleteChunk: entry chunk_id=%s', chunkId);
  const resp = await callCoreRpc<DeleteChunkResponse | ResultEnvelope<DeleteChunkResponse>>({
    method: 'openhuman.memory_tree_delete_chunk',
    params: { chunk_id: chunkId },
  });
  const out = unwrapResult(resp);
  console.debug(
    '[memory-tree-rpc] memoryTreeDeleteChunk: exit deleted=%o score_rows=%d entity_rows=%d',
    out?.deleted,
    out?.score_rows_removed,
    out?.entity_index_rows_removed
  );
  return out;
}

// ── memory_tree_get_llm / memory_tree_set_llm ────────────────────────────

/**
 * Read the currently configured LLM backend (`cloud` or `local`).
 */
export async function memoryTreeGetLlm(): Promise<LlmResponse> {
  console.debug('[memory-tree-rpc] memoryTreeGetLlm: entry');
  const resp = await callCoreRpc<LlmResponse | ResultEnvelope<LlmResponse>>({
    method: 'openhuman.memory_tree_get_llm',
  });
  const out = unwrapResult(resp);
  console.debug('[memory-tree-rpc] memoryTreeGetLlm: exit current=%s', out?.current);
  return out;
}

/**
 * Update the LLM backend selector — and, optionally, per-role model
 * choices (`cloud_model`, `extract_model`, `summariser_model`) — and
 * persist the result to `config.toml` in a single atomic write. Survives
 * sidecar restart.
 *
 * Returns the effective backend after the call (the core may downgrade
 * `local` → `cloud` if the host can't satisfy the local minimums; today
 * the handler accepts the value verbatim).
 *
 * Accepts either a bare backend string (legacy callers) or the full
 * {@link SetLlmRequest} object, so call-sites that only flip the mode
 * stay terse while sites that want to persist model picks pass the
 * extended shape.
 */
export async function memoryTreeSetLlm(
  reqOrBackend: LlmBackend | SetLlmRequest
): Promise<LlmResponse> {
  const params: SetLlmRequest =
    typeof reqOrBackend === 'string' ? { backend: reqOrBackend } : reqOrBackend;
  console.debug(
    '[memory-tree-rpc] memoryTreeSetLlm: entry backend=%s cloud_model=%s extract_model=%s summariser_model=%s',
    params.backend,
    params.cloud_model ?? '<none>',
    params.extract_model ?? '<none>',
    params.summariser_model ?? '<none>'
  );
  const resp = await callCoreRpc<LlmResponse | ResultEnvelope<LlmResponse>>({
    method: 'openhuman.memory_tree_set_llm',
    params,
  });
  const out = unwrapResult(resp);
  console.debug('[memory-tree-rpc] memoryTreeSetLlm: exit current=%s', out?.current);
  return out;
}

// ── memory_tree_graph_export ────────────────────────────────────────────

/**
 * Discriminator for graph nodes. `"summary"` is a sealed summary tree
 * node (Tree mode); `"chunk"` is a raw memory chunk and `"contact"`
 * is a person entity (Contacts mode).
 */
export type GraphNodeKind = 'summary' | 'chunk' | 'contact';

/**
 * One node in the graph export. Optional fields are populated only
 * when relevant to the node's `kind`; the UI branches on `kind` and
 * ignores the rest.
 */
export interface GraphNode {
  kind: GraphNodeKind;
  id: string;
  /** Display-friendly label (scope, preview snippet, or surface form). */
  label: string;

  // Summary-only ──
  tree_id?: string;
  tree_kind?: 'source' | 'topic' | 'global';
  tree_scope?: string;
  level?: number;
  parent_id?: string | null;
  child_count?: number;
  /** Filesystem-safe basename (no `.md`); used to build Obsidian deep links. */
  file_basename?: string;

  // Summary or chunk ──
  time_range_start_ms?: number;
  time_range_end_ms?: number;

  // Contact-only ──
  /** `"person" | "organization" | …`. */
  entity_kind?: string;
}

/** One explicit edge — used in Contacts mode to link chunks to contacts. */
export interface GraphEdge {
  from: string;
  to: string;
}

export type GraphMode = 'tree' | 'contacts';

export interface GraphExportResponse {
  nodes: GraphNode[];
  /**
   * Explicit edges. Empty in `tree` mode (each summary node's
   * `parent_id` carries the edge); chunk→contact mention edges in
   * `contacts` mode.
   */
  edges: GraphEdge[];
  /** Absolute filesystem path to `<workspace>/memory_tree/content/`. */
  content_root_abs: string;
}

/** Response shape for `memory_tree_wipe_all`. */
export interface WipeAllResponse {
  rows_deleted: number;
  dirs_removed: string[];
  /**
   * Composio sync-state KV rows deleted. Clearing these (per-connection
   * cursors + synced-id dedup sets) is what lets the next sync re-fetch
   * every upstream item instead of skipping ones it's already seen.
   */
  sync_state_cleared: number;
}

/**
 * Destructive reset: truncate every `mem_tree_*` table, remove the
 * on-disk chunk-store directories under the workspace content root,
 * **and** clear the `composio-sync-state` KV namespace so the next
 * sync re-fetches every upstream item from scratch (no
 * synced-id-dedup carry-over). Backed by
 * `openhuman.memory_tree_wipe_all`.
 *
 * Callers can rely on `sync_state_cleared` in the response — a
 * positive count means the next sync will be a full re-fetch; `0`
 * means there were no live cursors to drop (e.g. fresh workspace).
 */
export async function memoryTreeWipeAll(): Promise<WipeAllResponse> {
  console.debug('[memory-tree-rpc] memoryTreeWipeAll: entry');
  const resp = await callCoreRpc<WipeAllResponse | ResultEnvelope<WipeAllResponse>>({
    method: 'openhuman.memory_tree_wipe_all',
  });
  const out = unwrapResult(resp);
  console.debug(
    '[memory-tree-rpc] memoryTreeWipeAll: exit rows=%d dirs=%o',
    out.rows_deleted,
    out.dirs_removed
  );
  return out;
}

/** Response shape for `memory_tree_reset_tree`. */
export interface ResetTreeResponse {
  /** Tree-state SQLite rows deleted (summaries + trees + buffers + jobs). */
  tree_rows_deleted: number;
  /** Chunks reset to lifecycle_status = 'pending_extraction'. */
  chunks_requeued: number;
  /** `extract_chunk` jobs enqueued (one per chunk). */
  jobs_enqueued: number;
}

/**
 * Wipe summary-tree state but keep chunks, raw archive, and sync
 * state — then re-enqueue every chunk through extraction so the
 * tree rebuilds without a fresh upstream sync. Backed by
 * `openhuman.memory_tree_reset_tree`.
 *
 * Use after changing the summariser backend (e.g. flipping inert
 * → real local LLM) to re-summarise existing data on the new
 * model.
 */
export async function memoryTreeResetTree(): Promise<ResetTreeResponse> {
  console.debug('[memory-tree-rpc] memoryTreeResetTree: entry');
  const resp = await callCoreRpc<ResetTreeResponse | ResultEnvelope<ResetTreeResponse>>({
    method: 'openhuman.memory_tree_reset_tree',
  });
  const out = unwrapResult(resp);
  console.debug(
    '[memory-tree-rpc] memoryTreeResetTree: exit tree_rows=%d chunks=%d jobs=%d',
    out.tree_rows_deleted,
    out.chunks_requeued,
    out.jobs_enqueued
  );
  return out;
}

/** Response shape for `memory_tree_flush_now`. */
export interface FlushNowResponse {
  enqueued: boolean;
  stale_buffers: number;
}

/**
 * Manually trigger the summary-tree build. Enqueues a `flush_stale` job
 * with `max_age_secs=0` so every L0 buffer force-seals immediately; the
 * seal worker runs each through the configured cloud or local
 * summariser. Backed by `openhuman.memory_tree_flush_now`.
 *
 * Safe to spam — same UTC-day dedupe key as the scheduled flush, so
 * duplicate clicks return `enqueued=false` rather than queuing twice.
 */
export async function memoryTreeFlushNow(): Promise<FlushNowResponse> {
  console.debug('[memory-tree-rpc] memoryTreeFlushNow: entry');
  const resp = await callCoreRpc<FlushNowResponse | ResultEnvelope<FlushNowResponse>>({
    method: 'openhuman.memory_tree_flush_now',
  });
  const out = unwrapResult(resp);
  console.debug(
    '[memory-tree-rpc] memoryTreeFlushNow: exit enqueued=%s stale_buffers=%d',
    out.enqueued,
    out.stale_buffers
  );
  return out;
}

/**
 * Return either the summary tree (parent→child links between sealed
 * summaries) or the document↔contact graph (chunks linked to person
 * entities they mention). Backed by `openhuman.memory_tree_graph_export`.
 */
export async function memoryTreeGraphExport(
  mode: GraphMode = 'tree'
): Promise<GraphExportResponse> {
  console.debug('[memory-tree-rpc] memoryTreeGraphExport: entry mode=%s', mode);
  const resp = await callCoreRpc<GraphExportResponse | ResultEnvelope<GraphExportResponse>>({
    method: 'openhuman.memory_tree_graph_export',
    params: { mode },
  });
  const out = unwrapResult(resp);
  console.debug(
    // Don't log the absolute content root — it embeds the user's
    // home directory + username and shows up in console logs / bug
    // reports. The path is still returned to the caller.
    '[memory-tree-rpc] memoryTreeGraphExport: exit mode=%s n=%d edges=%d',
    mode,
    out.nodes?.length ?? 0,
    out.edges?.length ?? 0
  );
  return out;
}
