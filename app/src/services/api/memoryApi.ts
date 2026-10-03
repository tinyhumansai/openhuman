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
  | 'composio'
  | 'conversation'
  | 'agent'
  | 'import';

/** The source kinds a user can register as a synced Documents source. */
export type DocumentSourceKind = 'folder' | 'file' | 'link' | 'github' | 'rss' | 'composio';

export const DOCUMENT_SOURCE_KINDS: readonly DocumentSourceKind[] = [
  'folder',
  'file',
  'link',
  'github',
  'rss',
  'composio',
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
}

export interface ItemsPage {
  items: Hit[];
  next_cursor?: string | null;
}

export interface RecentConversation {
  thread_id: string;
  turns: number;
  stored_at: string;
}

export interface ConversationsSettings {
  enabled: boolean;
  batch_turns: number;
  idle_secs: number;
  recent: RecentConversation[];
}

export interface ConversationsUpdate {
  enabled?: boolean;
  batch_turns?: number;
  idle_secs?: number;
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
}

export interface MemoryContext {
  markdown: string;
  tokens: number;
  generated_at: string | null;
  interval_mins: number;
  budget_tokens: number;
  enabled: boolean;
}

export interface ContextUpdate {
  enabled?: boolean;
  interval_mins?: number;
  budget_tokens?: number;
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

export interface ImportState {
  phase: ImportPhase;
  imported: number;
  total: number;
  error?: string | null;
}

/** The structured error codes a memory RPC can fail with. */
export type MemoryErrorCode =
  | 'MEMORY_OFF'
  | 'UNSUPPORTED'
  | 'INVALID_REQUEST'
  | 'UNAUTHORIZED'
  | 'ENGINE';

const MEMORY_ERROR_CODES: readonly MemoryErrorCode[] = [
  'MEMORY_OFF',
  'UNSUPPORTED',
  'INVALID_REQUEST',
  'UNAUTHORIZED',
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
  const message = (err as { message?: unknown }).message;
  if (typeof message === 'string') {
    const match = /^\s*(MEMORY_OFF|UNSUPPORTED|INVALID_REQUEST|UNAUTHORIZED|ENGINE)\b/.exec(
      message
    );
    if (match) return match[1] as MemoryErrorCode;
  }
  return null;
}

/** Human-readable text of any thrown value. */
export function memoryErrorMessage(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (err && typeof err === 'object' && 'message' in err) return String(err.message);
  return String(err);
}

/** True when the engine state means memory is usable (an engine is set and not off). */
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

export function memoryItemsList(req: ItemsListRequest = {}): Promise<ItemsPage> {
  return call<ItemsPage>(CORE_RPC_METHODS.memoryItemsList, req);
}

// ─── Conversations ───────────────────────────────────────────────────────────

export function memoryConversationsGet(): Promise<ConversationsSettings> {
  return call<ConversationsSettings>(CORE_RPC_METHODS.memoryConversationsGet);
}

export function memoryConversationsSet(
  update: ConversationsUpdate
): Promise<ConversationsSettings> {
  return call<ConversationsSettings>(CORE_RPC_METHODS.memoryConversationsSet, update);
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

// ─── context.md ──────────────────────────────────────────────────────────────

export function memoryContextGet(): Promise<MemoryContext> {
  return call<MemoryContext>(CORE_RPC_METHODS.memoryContextGet);
}

export function memoryContextRefresh(): Promise<MemoryContext> {
  return call<MemoryContext>(CORE_RPC_METHODS.memoryContextRefresh);
}

export function memoryContextSet(update: ContextUpdate): Promise<MemoryContext> {
  return call<MemoryContext>(CORE_RPC_METHODS.memoryContextSet, update);
}

// ─── Import of previous (v1) memory ──────────────────────────────────────────

export function memoryImportScan(): Promise<ImportScan> {
  return call<ImportScan>(CORE_RPC_METHODS.memoryImportScan);
}

/** Start the upload of local v1 data to the selected engine. Requires explicit consent. */
export function memoryImportStart(): Promise<{ state: ImportState }> {
  return call<{ state: ImportState }>(CORE_RPC_METHODS.memoryImportStart, { consent: true });
}

export function memoryImportStatus(): Promise<{ state: ImportState }> {
  return call<{ state: ImportState }>(CORE_RPC_METHODS.memoryImportStatus);
}
