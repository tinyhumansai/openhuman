/**
 * Memory v2 API — the typed facade over the `openhuman.memory_*` RPC surface.
 *
 * One file per domain: the Memory page and every component under
 * `components/memory/` call these functions and never `callCoreRpc` directly.
 * The wire shapes are the accepted spec's (`docs/specs/memory-v2.md`) and stay
 * in snake_case, so the types here mirror the JSON exactly.
 *
 * Errors come back as the standard structured RPC error; {@link memoryErrorCode}
 * reads its `code` (`MEMORY_OFF`, `UNSUPPORTED`, `INVALID_REQUEST`,
 * `UNAUTHORIZED`, `ENGINE`) so callers can branch without parsing messages.
 *
 * debug logging: DEBUG=openhuman:memoryApi
 */
import debug from 'debug';

import { callCoreRpc } from '../coreRpcClient';
import { CORE_RPC_METHODS } from '../rpcMethods';

const log = debug('openhuman:memoryApi');

// ─── Domain types ────────────────────────────────────────────────────────────

/** How a fetch searches. An engine lists the subset it supports. */
export type FetchMode = 'keyword' | 'vector' | 'hybrid';

/** The three kinds of stored item. */
export type ItemKind = 'document' | 'conversation' | 'learning';

/** Where an item came from. */
export type SourceKind =
  | 'folder'
  | 'file'
  | 'link'
  | 'github'
  | 'rss'
  | 'conversation'
  | 'agent'
  | 'import';

/** The source kinds a user can register as a synced Documents source. */
export type DocumentSourceKind = 'folder' | 'file' | 'link' | 'github' | 'rss';

export const DOCUMENT_SOURCE_KINDS: readonly DocumentSourceKind[] = [
  'folder',
  'file',
  'link',
  'github',
  'rss',
];

/** The kinds a learning can be stored as. */
export type LearningKind = 'preference' | 'fact' | 'procedure' | 'correction' | 'other';

export const LEARNING_KINDS: readonly LearningKind[] = [
  'preference',
  'fact',
  'procedure',
  'correction',
  'other',
];

/**
 * The tag TinyMemory's belief builder stamps on the learnings it derives
 * (`tinymemory_api::consolidate::BELIEF_TAG`).
 */
export const BELIEF_TAG = 'belief';

/** True when a stored item is a belief the background builder derived. */
export function isBuiltBelief(meta: { tags?: string[] | null } | null | undefined): boolean {
  return Boolean(meta?.tags?.includes(BELIEF_TAG));
}

/** Engine health as `memory_engine_get` reports it. `off` = no usable engine. */
export type EngineStatus = 'ok' | 'degraded' | 'down' | 'off';

export interface EngineDescriptor {
  id: string;
  label: string;
  description: string;
  hosted: boolean;
  needs_endpoint: boolean;
  needs_key: boolean;
  default_endpoint?: string | null;
  fetch_modes: FetchMode[];
}

export interface EnginesList {
  engines: EngineDescriptor[];
  active: string | null;
}

export interface EngineState {
  engine: string | null;
  endpoint?: string;
  has_key: boolean;
  status: EngineStatus;
  reason?: string;
  fetch_modes: FetchMode[];
}

export interface EngineSetRequest {
  engine: string;
  endpoint?: string;
  api_key?: string;
}

export interface MemorySourceRef {
  kind: SourceKind;
  id?: string | null;
}

export interface TurnRange {
  first: number;
  last: number;
}

export interface ToolCallRef {
  name: string;
  id: string;
}

/** Item metadata, TinyMemory field names in snake_case. Every field is optional on the wire. */
export interface MemoryMeta {
  workspace?: string | null;
  folder?: string | null;
  file_path?: string | null;
  language?: string | null;
  repo?: string | null;
  commit?: string | null;
  url?: string | null;
  thread_id?: string | null;
  turns?: TurnRange | null;
  agent_id?: string | null;
  tool_call?: ToolCallRef | null;
  source?: MemorySourceRef | null;
  tags?: string[];
  observed_at?: string | null;
}

/** A metadata filter: the meta fields as exact matches plus the list/window fields. */
export interface MetaFilter {
  workspace?: string;
  folder?: string;
  file_path?: string;
  language?: string;
  repo?: string;
  commit?: string;
  url?: string;
  thread_id?: string;
  agent_id?: string;
  kinds?: ItemKind[];
  sources?: SourceKind[];
  tags_any?: string[];
  observed_after?: string;
  observed_before?: string;
}

export interface Hit {
  id: string;
  kind: ItemKind;
  text: string;
  meta: MemoryMeta;
  score: number;
}

/** A recall citation: a {@link Hit} with `snippet` in place of `text`. */
export interface Citation {
  id: string;
  kind: ItemKind;
  snippet: string;
  meta: MemoryMeta;
  score?: number | null;
}

export interface RecallRequest {
  question: string;
  filter?: MetaFilter;
  limit?: number;
}

export interface RecallAnswer {
  answer: string;
  citations: Citation[];
  model?: string | null;
}

export interface FetchRequest {
  query: string;
  mode?: FetchMode;
  filter?: MetaFilter;
  limit?: number;
  cursor?: string;
}

export interface FetchPage {
  hits: Hit[];
  next_cursor?: string | null;
}

export interface LearnRequest {
  text: string;
  kind?: LearningKind;
  confidence?: number;
  meta?: Partial<MemoryMeta>;
}

export interface ItemsListRequest {
  filter?: MetaFilter;
  limit?: number;
  cursor?: string;
  /** Explorer path; the core narrows `filter` by each step. */
  path?: PathStep[];
  /**
   * Snippet listing: a conversation's or chunked document's `text` may be
   * only its start, which the engine lists without assembling each item.
   * Read an item whole with `memoryItemsGet` when it is opened.
   */
  preview?: boolean;
}

// ─── Explorer ────────────────────────────────────────────────────────────────

/**
 * A metadata dimension the explorer groups by — TinyMemory's standard facets,
 * the same for every engine.
 */
export type Facet =
  | 'kind'
  | 'source'
  | 'source_id'
  | 'workspace'
  | 'folder'
  | 'file_path'
  | 'language'
  | 'repo'
  | 'url'
  | 'thread'
  | 'agent'
  | 'tool_call'
  | 'tag'
  | 'namespace';

export const FACETS: readonly Facet[] = [
  'kind',
  'namespace',
  'source',
  'source_id',
  'workspace',
  'folder',
  'file_path',
  'language',
  'repo',
  'url',
  'thread',
  'agent',
  'tool_call',
  'tag',
];

/** One step down the explorer: the items whose `facet` is `value`. */
export interface PathStep {
  facet: Facet;
  value: string;
}

export interface ExploreRequest {
  facet: Facet;
  path?: PathStep[];
  filter?: MetaFilter;
  limit?: number;
  scan_limit?: number;
}

export interface FacetBucket {
  value: string;
  count: number;
}

export interface ExplorePage {
  facet: Facet;
  /** Largest first. */
  buckets: FacetBucket[];
  /** Items under the path (that were read, when `truncated`). */
  total: number;
  /** Of those, items with no value for the facet. */
  missing: number;
  /** Values left out by the bucket limit. */
  more_buckets: number;
  /** The engine stopped scanning early, so counts are a lower bound. */
  truncated: boolean;
}

export interface ItemsPage {
  items: Hit[];
  next_cursor?: string | null;
}

export type SourceStatus = 'idle' | 'syncing' | 'error';

export interface Source {
  id: string;
  kind: DocumentSourceKind;
  target: string;
  label: string;
  schedule_mins?: number | null;
  last_sync_at?: string | null;
  status: SourceStatus;
  error?: string | null;
  items: number;
}

export interface SourceAddRequest {
  kind: DocumentSourceKind;
  target: string;
  label?: string;
  schedule_mins?: number;
  /** Layout root to file the documents under, e.g. `team:acme`. */
  namespace?: string;
}

export interface ImportCounts {
  documents: number;
  conversations: number;
  learnings: number;
}

export interface ImportScan {
  found: boolean;
  counts?: ImportCounts | null;
}

export type ImportPhase = 'idle' | 'running' | 'done' | 'error';

/** `memory_migration_scan`: whether memory from before the per-user layout is left to move. */
export interface MigrationScan {
  needed: boolean;
  /** The legacy tree may be shared with other accounts here; moving it needs consent. */
  shared: boolean;
}

export type MigrationPhase = 'idle' | 'copying' | 'paused' | 'copied' | 'cleaning' | 'cleaned';

export interface MigrationState {
  phase: MigrationPhase;
  copied: number;
  /** Items that could not be moved; they stay in the legacy tree. */
  failures?: { id: string; reason: string }[];
  /** Items the legacy tree holds only part of. */
  incomplete?: string[];
  error?: string | null;
}

export interface MigrationStatus {
  state: MigrationState;
  running: boolean;
  interrupted: boolean;
}

export interface ImportState {
  phase: ImportPhase;
  imported: number;
  total: number;
  error?: string | null;
  /** Items the engine refused; `memoryImportRetryFailed` stores them again. */
  failed?: number;
}

/** Progress of storing past chats (`memory_conversations_backfill_*`). */
export interface BackfillState {
  phase: ImportPhase;
  threads_total: number;
  threads_done: number;
  turns_stored: number;
  items_stored: number;
  error?: string | null;
  finished_at?: string | null;
}

export interface BackfillView {
  state: BackfillState;
  /** Threads that still have turns from before automatic saving. */
  pending_threads: number;
  /** Turns still to store across them. */
  pending_turns: number;
}

// ─── Lifecycle policy ────────────────────────────────────────────────────────

/** What the memory pack injected before every turn may hold, and when beliefs build. */
export interface RecallPolicy {
  enabled: boolean;
  /** Token budget of one pack. */
  budget_tokens: number;
  learnings_limit: number;
  brain_limit: number;
  history_limit: number;
  team_limit: number;
  /** Build beliefs every N logged turns; `0` turns building off. */
  build_beliefs_every: number;
  pre_turn_timeout_ms: number;
  compaction_timeout_ms: number;
  build_delay_secs: number;
}

/** `memory_policy_get` / `memory_policy_set`. */
export interface MemoryPolicy {
  /** Log every turn (pre_turn / post_turn) to memory. */
  log_conversations: boolean;
  recall: RecallPolicy;
  /** The layout root this identity's memory is filed under. */
  root: string;
  /** The memory agent id turns run as. */
  agent_id: string;
  /** The host pins the root/agent (e.g. an embedder), so they cannot change here. */
  host_bound: boolean;
}

/** A partial policy update. Unknown keys are rejected by the core. */
export interface PolicyUpdate {
  log_conversations?: boolean;
  recall_enabled?: boolean;
  /** 100–16000. */
  budget_tokens?: number;
  /** 0–50 each. */
  learnings_limit?: number;
  brain_limit?: number;
  history_limit?: number;
  team_limit?: number;
  /** 0–1000; 0 = off. */
  build_beliefs_every?: number;
  /** 100–30000. */
  pre_turn_timeout_ms?: number;
}

// ─── Memory pack ─────────────────────────────────────────────────────────────

export interface PackPreviewRequest {
  /** A turn's text; without it the preview is a session start. */
  query?: string;
  thread_id?: string;
  agent_id?: string;
}

export interface PackSection {
  heading: string;
  answer?: string | null;
  hits: Hit[];
}

export interface PackSkipped {
  heading: string;
  reason: string;
}

export interface MemoryPack {
  markdown: string;
  tokens: number;
  refs: string[];
  sections: PackSection[];
  skipped: PackSkipped[];
  engine: string;
}

export interface PackPreview {
  agent_id: string;
  root: string;
  mode: 'turn' | 'session';
  pack: MemoryPack;
}

export interface MemoryAgent {
  agent_id: string;
  /** Turns logged for this agent. */
  turns: number;
}

export interface AgentsList {
  root: string;
  agents: MemoryAgent[];
}

// ─── Brain (shared documents) ────────────────────────────────────────────────

/** Documents filed under one source type (pdf, markdown, notion, github, web, gmail, …). */
export interface BrainSource {
  source: string;
  documents: number;
}

export interface BrainSources {
  root: string;
  sources: BrainSource[];
  /** Documents with no source type. */
  unfiled: number;
}

export interface BrainSearchRequest {
  query: string;
  source?: string;
  limit?: number;
}

/** Exactly one of `path` / `text`. */
export interface BrainIngestRequest {
  path?: string;
  text?: string;
  source?: string;
  title?: string;
}

export interface BrainIngestResult {
  id: string;
  source: string;
  /** The same document was already in the brain; nothing new was stored. */
  replayed: boolean;
}

// ─── Background jobs ─────────────────────────────────────────────────────────

export type MemoryJobKind = 'build_beliefs' | 'ingest_brain';

export interface PendingJob {
  id: string;
  root: string;
  job: { job: MemoryJobKind | string; [key: string]: unknown };
  queued_at: string;
  attempts: number;
  last_error?: string | null;
}

export type JobOutcome = 'done' | 'started' | 'scheduled' | 'skipped' | 'failed';

export interface JobRun {
  id: string;
  job: string;
  root: string;
  ran_at: string;
  outcome: JobOutcome;
  reason?: string | null;
  built?: number | null;
  stored: number;
}

export interface JobsList {
  pending: PendingJob[];
  history: JobRun[];
}

/**
 * A plan refusal's code in the head TinyMemory gives a hosted failure,
 * `[MEMORY_PLAN_REQUIRED] memory API …` (#8081). Delimited, so a message that
 * merely mentions the code does not match.
 */
const PLAN_REFUSAL = /\[[A-Z0-9_]*_PLAN_REQUIRED\] memory API /;

/** The structured error codes a memory RPC can fail with. */
export type MemoryErrorCode =
  | 'MEMORY_OFF'
  | 'UNSUPPORTED'
  | 'INVALID_REQUEST'
  | 'UNAUTHORIZED'
  | 'INSUFFICIENT_CREDITS'
  | 'PLAN_REQUIRED'
  | 'UNAVAILABLE'
  | 'ENGINE';

const MEMORY_ERROR_CODES: readonly MemoryErrorCode[] = [
  'MEMORY_OFF',
  'UNSUPPORTED',
  'INVALID_REQUEST',
  'UNAUTHORIZED',
  'INSUFFICIENT_CREDITS',
  'PLAN_REQUIRED',
  'UNAVAILABLE',
  'ENGINE',
];

// ─── Helpers ─────────────────────────────────────────────────────────────────

/** Unwrap the controller's `{ result, logs }` envelope when the core sends one. */
function unwrap<T>(raw: unknown): T {
  if (raw && typeof raw === 'object' && !Array.isArray(raw) && 'result' in raw) {
    const keys = Object.keys(raw as Record<string, unknown>);
    // Only the envelope has `result` beside at most `logs`; a payload that
    // legitimately carries a `result` field would have other keys too.
    if (keys.every(k => k === 'result' || k === 'logs')) {
      return (raw as { result: T }).result;
    }
  }
  return raw as T;
}

/** Drop `undefined` params so the wire payload stays clean. */
function prune<T extends object>(params: T): Partial<T> {
  const out: Partial<T> = {};
  for (const [k, v] of Object.entries(params)) {
    if (v !== undefined) (out as Record<string, unknown>)[k] = v;
  }
  return out;
}

async function call<T>(method: string, params: object = {}): Promise<T> {
  log('%s: request', method);
  try {
    const raw = await callCoreRpc<unknown>({ method, params: prune(params) });
    log('%s: ok', method);
    return unwrap<T>(raw);
  } catch (err) {
    log('%s: failed code=%s', method, memoryErrorCode(err) ?? 'none');
    throw err;
  }
}

/**
 * The structured `code` of a failed memory RPC, or `null` when the error does
 * not carry one. Reads `error.data.code` (or `data.kind`), then falls back to a
 * `CODE:` / `CODE ` prefix on the message for a core that only sends text.
 */
export function memoryErrorCode(err: unknown): MemoryErrorCode | null {
  if (!err || typeof err !== 'object') return null;
  const message = (err as { message?: unknown }).message;
  // Before the structured code: an older core files a plan refusal under
  // `UNAUTHORIZED`, with the backend's code still on the message.
  if (typeof message === 'string' && PLAN_REFUSAL.test(message)) return 'PLAN_REQUIRED';
  const data = (err as { data?: unknown }).data;
  if (data && typeof data === 'object') {
    const d = data as Record<string, unknown>;
    for (const field of ['code', 'kind']) {
      const v = d[field];
      if (typeof v === 'string' && (MEMORY_ERROR_CODES as readonly string[]).includes(v)) {
        return v as MemoryErrorCode;
      }
    }
  }
  if (typeof message === 'string') {
    const match =
      /^\s*(MEMORY_OFF|UNSUPPORTED|INVALID_REQUEST|UNAUTHORIZED|INSUFFICIENT_CREDITS|PLAN_REQUIRED|UNAVAILABLE|ENGINE)\b/.exec(
        message
      );
    if (match) return match[1] as MemoryErrorCode;
  }
  return null;
}

/**
 * The account-wide refusals whose raw message is engine detail, each told
 * apart from an engine fault and from an empty memory: out of credits means
 * "top up", not in the plan means "upgrade", unreachable means "try again". `UNAUTHORIZED` keeps its own
 * message, which names the rejected key or session.
 */
const REFUSAL_KEYS: Partial<Record<MemoryErrorCode, string>> = {
  INSUFFICIENT_CREDITS: 'memory.error.insufficientCredits',
  PLAN_REQUIRED: 'memory.error.planRequired',
  UNAVAILABLE: 'memory.error.unavailable',
};

/**
 * Human-readable text of any thrown value. Given `t`, an account-wide refusal
 * reads as its translated explanation instead of the engine's raw message.
 */
export function memoryErrorMessage(
  err: unknown,
  t?: (key: string, fallback?: string) => string
): string {
  const code = memoryErrorCode(err);
  const key = code ? REFUSAL_KEYS[code] : undefined;
  if (t && key) return t(key);
  if (err instanceof Error) return err.message;
  if (err && typeof err === 'object' && 'message' in err) return String(err.message);
  return String(err);
}

/**
 * True when `message` is the out-of-credits explanation `memoryErrorMessage`
 * produced (with `t`). That translated text is returned for
 * `INSUFFICIENT_CREDITS` and for nothing else, so the views that keep only the
 * message can still offer a top-up instead of an error.
 *
 * Known edge: a message produced in one language no longer matches after the
 * user switches language, so that stale message falls back to the error alert
 * (it still explains the top-up). The next failed action re-derives it.
 */
export function isOutOfCreditsMessage(
  message: string | null,
  t: (key: string, fallback?: string) => string
): boolean {
  const key = REFUSAL_KEYS.INSUFFICIENT_CREDITS;
  return message !== null && key !== undefined && message === t(key);
}

/**
 * A stored status error (a job, sync source, import, migration or backfill that
 * stopped) for display. A plan refusal arrives as the engine's raw
 * `[MEMORY_PLAN_REQUIRED] memory API …` text; it reads as its translated
 * explanation instead (#8081). Anything else is shown as stored.
 */
export function memoryStatusError(
  raw: string,
  t: (key: string, fallback?: string) => string
): string {
  const key = REFUSAL_KEYS.PLAN_REQUIRED;
  return key && PLAN_REFUSAL.test(raw) ? t(key) : raw;
}

/**
 * True when `message` is the "memory is not in your plan" explanation
 * `memoryErrorMessage` produced (with `t`) for `PLAN_REQUIRED` (#8081). Same
 * contract and language edge as {@link isOutOfCreditsMessage}.
 */
export function isPlanRequiredMessage(
  message: string | null,
  t: (key: string, fallback?: string) => string
): boolean {
  const key = REFUSAL_KEYS.PLAN_REQUIRED;
  return message !== null && key !== undefined && message === t(key);
}

/** True when the engine state means memory is usable (an engine is set and not off). */
/** Engine id that turns memory off completely (`memory_engine_set`). */
export const MEMORY_DISABLED_ENGINE = 'none';

/** Whether memory was turned off on purpose (the `none` engine). */
export function isMemoryDisabled(state: EngineState | null | undefined): boolean {
  return state?.engine === MEMORY_DISABLED_ENGINE;
}

export function isMemoryOn(state: EngineState | null | undefined): boolean {
  return Boolean(state && state.engine && state.status !== 'off');
}

// ─── Engine ──────────────────────────────────────────────────────────────────

export function memoryEnginesList(): Promise<EnginesList> {
  return call<EnginesList>(CORE_RPC_METHODS.memoryEnginesList);
}

export function memoryEngineGet(): Promise<EngineState> {
  return call<EngineState>(CORE_RPC_METHODS.memoryEngineGet);
}

export function memoryEngineSet(req: EngineSetRequest): Promise<EngineState> {
  return call<EngineState>(CORE_RPC_METHODS.memoryEngineSet, req);
}

// ─── Recall / fetch / store ──────────────────────────────────────────────────

export function memoryRecall(req: RecallRequest): Promise<RecallAnswer> {
  return call<RecallAnswer>(CORE_RPC_METHODS.memoryRecall, req);
}

export function memoryFetch(req: FetchRequest): Promise<FetchPage> {
  return call<FetchPage>(CORE_RPC_METHODS.memoryFetch, req);
}

export function memoryLearn(req: LearnRequest): Promise<{ id: string }> {
  return call<{ id: string }>(CORE_RPC_METHODS.memoryLearn, req);
}

export function memoryForget(ids: string[]): Promise<{ forgotten: number }> {
  return call<{ forgotten: number }>(CORE_RPC_METHODS.memoryForget, { ids });
}

/**
 * Erase the user's entire memory, for good: every source, conversation and
 * fact the bound engine holds for this account. The core refuses without
 * `confirm: true`, so the interlock is always sent explicitly here.
 */
export function memoryEraseAll(): Promise<{ erased_scopes: number }> {
  return call<{ erased_scopes: number }>(CORE_RPC_METHODS.memoryEraseAll, { confirm: true });
}

export function memoryItemsList(req: ItemsListRequest = {}): Promise<ItemsPage> {
  return call<ItemsPage>(CORE_RPC_METHODS.memoryItemsList, req);
}

export function memoryExplore(req: ExploreRequest): Promise<ExplorePage> {
  return call<ExplorePage>(CORE_RPC_METHODS.memoryExplore, req);
}

/** Items read whole, in the order asked; unknown ids are left out. */
export function memoryItemsGet(ids: string[]): Promise<{ items: Hit[] }> {
  return call<{ items: Hit[] }>(CORE_RPC_METHODS.memoryItemsGet, { ids });
}

// ─── Documents (sources) ─────────────────────────────────────────────────────

export function memorySourcesList(): Promise<{ sources: Source[] }> {
  return call<{ sources: Source[] }>(CORE_RPC_METHODS.memorySourcesList);
}

export function memorySourcesAdd(req: SourceAddRequest): Promise<{ source: Source }> {
  return call<{ source: Source }>(CORE_RPC_METHODS.memorySourcesAdd, req);
}

export function memorySourcesRemove(
  id: string,
  forgetItems?: boolean
): Promise<{ removed: boolean }> {
  return call<{ removed: boolean }>(CORE_RPC_METHODS.memorySourcesRemove, {
    id,
    forget_items: forgetItems,
  });
}

/** Sync one source, or every source when `id` is omitted. */
export function memorySourcesSync(id?: string): Promise<{ started: string[] }> {
  return call<{ started: string[] }>(CORE_RPC_METHODS.memorySourcesSync, { id });
}

// ─── Lifecycle policy and the memory pack ────────────────────────────────────

export function memoryPolicyGet(): Promise<MemoryPolicy> {
  return call<MemoryPolicy>(CORE_RPC_METHODS.memoryPolicyGet);
}

export function memoryPolicySet(update: PolicyUpdate): Promise<MemoryPolicy> {
  return call<MemoryPolicy>(CORE_RPC_METHODS.memoryPolicySet, update);
}

/** The pack a turn would get (with `query`) or a session start (without). */
export function memoryPackPreview(req: PackPreviewRequest = {}): Promise<PackPreview> {
  return call<PackPreview>(CORE_RPC_METHODS.memoryPackPreview, req);
}

/** The agents with logged turns under this identity's root. */
export function memoryAgentsList(): Promise<AgentsList> {
  return call<AgentsList>(CORE_RPC_METHODS.memoryAgentsList);
}

// ─── Brain (shared documents) ────────────────────────────────────────────────

export function memoryBrainSources(): Promise<BrainSources> {
  return call<BrainSources>(CORE_RPC_METHODS.memoryBrainSources);
}

export function memoryBrainSearch(req: BrainSearchRequest): Promise<{ hits: Hit[] }> {
  return call<{ hits: Hit[] }>(CORE_RPC_METHODS.memoryBrainSearch, req);
}

export function memoryBrainIngest(req: BrainIngestRequest): Promise<BrainIngestResult> {
  return call<BrainIngestResult>(CORE_RPC_METHODS.memoryBrainIngest, req);
}

/** Forget every brain document filed under `source`. */
export function memoryBrainForget(source: string): Promise<{ forgotten: number }> {
  return call<{ forgotten: number }>(CORE_RPC_METHODS.memoryBrainForget, { source });
}

// ─── Background jobs ─────────────────────────────────────────────────────────

export function memoryJobsList(): Promise<JobsList> {
  return call<JobsList>(CORE_RPC_METHODS.memoryJobsList);
}

/** Run one pending job now, or every pending job when `id` is omitted. */
export function memoryJobsRun(id?: string): Promise<{ runs: JobRun[] }> {
  return call<{ runs: JobRun[] }>(CORE_RPC_METHODS.memoryJobsRun, { id });
}

// ─── Import of previous (v1) memory ──────────────────────────────────────────

export function memoryImportScan(): Promise<ImportScan> {
  return call<ImportScan>(CORE_RPC_METHODS.memoryImportScan);
}

/** Start the upload of local v1 data to the selected engine. Requires explicit consent. */
export function memoryImportStart(): Promise<{ state: ImportState }> {
  return call<{ state: ImportState }>(CORE_RPC_METHODS.memoryImportStart, { consent: true });
}

export function memoryConversationsBackfillStatus(): Promise<BackfillView> {
  return call<BackfillView>(CORE_RPC_METHODS.memoryConversationsBackfillStatus);
}

/** Starts storing past chats. Only the consent dialog calls this. */
export function memoryConversationsBackfillStart(): Promise<BackfillView> {
  return call<BackfillView>(CORE_RPC_METHODS.memoryConversationsBackfillStart, { consent: true });
}

export function memoryImportStatus(): Promise<{ state: ImportState }> {
  return call<{ state: ImportState }>(CORE_RPC_METHODS.memoryImportStatus);
}

/** Stores again the items a finished import skipped because the engine refused them. */
export function memoryImportRetryFailed(): Promise<{ state: ImportState }> {
  return call<{ state: ImportState }>(CORE_RPC_METHODS.memoryImportRetryFailed);
}

// ─── Move into the per-user layout ───────────────────────────────────────────

export function memoryMigrationScan(): Promise<MigrationScan> {
  return call<MigrationScan>(CORE_RPC_METHODS.memoryMigrationScan);
}

/** "Migrate now". `takeover` is the user's consent to take a tree other accounts may share. */
export function memoryMigrationStart(takeover = false): Promise<MigrationStatus> {
  return call<MigrationStatus>(CORE_RPC_METHODS.memoryMigrationStart, { takeover });
}

export function memoryMigrationStatus(): Promise<MigrationStatus> {
  return call<MigrationStatus>(CORE_RPC_METHODS.memoryMigrationStatus);
}

/** Puts the items that could not be moved back in line for the next run. */
export function memoryMigrationRetry(): Promise<MigrationState> {
  return call<MigrationState>(CORE_RPC_METHODS.memoryMigrationRetry);
}
