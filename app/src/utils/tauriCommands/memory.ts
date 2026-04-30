/**
 * Memory subsystem commands.
 */
import { callCoreRpc } from '../../services/coreRpcClient';
import { isTauri } from './common';

export interface MemoryDebugDocument {
  documentId: string;
  namespace: string;
  title?: string;
  raw: unknown;
}

/** A single entity returned in the structured retrieval context. */
export interface MemoryRetrievalEntity {
  id?: string;
  name: string;
  entity_type?: string;
  score?: number;
  metadata?: unknown;
}

/** Structured retrieval context returned alongside `llm_context_message`. */
export interface MemoryRetrievalContext {
  entities: MemoryRetrievalEntity[];
  relations: { subject: string; predicate: string; object: string; score?: number }[];
  chunks: { content: string; score: number; chunk_id?: string; document_id?: string }[];
}

/** Result of a memory query or recall, combining text and structured data. */
export interface MemoryQueryResult {
  text: string;
  entities: MemoryRetrievalEntity[];
}

/**
 * Raw envelope shape returned by `openhuman.memory_query_namespace` and
 * `openhuman.memory_recall_context` via the registry-based RPC handler.
 */
interface MemoryQueryEnvelope {
  data?: { llm_context_message?: string | null; context?: MemoryRetrievalContext | null };
  llm_context_message?: string | null;
  context?: MemoryRetrievalContext | null;
}

/** Extract text + entities from the envelope returned by query/recall RPCs. */
function unwrapMemoryQueryResult(resp: unknown): MemoryQueryResult {
  // If the response is already a plain string, return it directly.
  if (typeof resp === 'string') {
    return { text: resp, entities: [] };
  }

  const envelope = resp as MemoryQueryEnvelope | null;
  if (!envelope || typeof envelope !== 'object') {
    return { text: '', entities: [] };
  }

  // Envelope may be `{ data: { llm_context_message, context } }` or flat.
  const inner = envelope.data ?? envelope;
  const text = inner.llm_context_message ?? '';
  const entities = inner.context?.entities ?? [];

  return { text, entities };
}

export interface GraphRelation {
  namespace: string | null;
  subject: string;
  predicate: string;
  object: string;
  attrs: Record<string, unknown>;
  updatedAt: number;
  evidenceCount: number;
  orderIndex: number | null;
  documentIds: string[];
  chunkIds: string[];
}

/**
 * Initialise the local-only (SQLite) memory subsystem in the Rust core.
 */
export async function syncMemoryClientToken(token: string): Promise<void> {
  console.debug(
    '[memory] syncMemoryClientToken: entry (token_present=%s, is_tauri=%s)',
    !!token,
    isTauri()
  );
  if (!isTauri()) {
    console.debug('[memory] syncMemoryClientToken: exit — skipped (not Tauri)');
    return;
  }
  try {
    console.debug('[memory] syncMemoryClientToken: payload → memory.init (local-only)');
    // jwt_token is passed for backward compatibility but ignored by the core.
    await callCoreRpc<boolean>({ method: 'openhuman.memory_init', params: { jwt_token: token } });
    console.info('[memory] syncMemoryClientToken: exit — ok');
  } catch (err) {
    console.warn('[memory] syncMemoryClientToken: exit — error:', err);
  }
}

export async function memoryListDocuments(namespace?: string): Promise<unknown> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  const resp = await callCoreRpc<unknown>({
    method: 'openhuman.memory_list_documents',
    params: { namespace },
  });
  // Unwrap envelope: registry returns { data: { documents: [...] }, meta: {...} }
  if (resp && typeof resp === 'object' && !Array.isArray(resp) && 'data' in resp) {
    return (resp as Record<string, unknown>).data;
  }
  return resp;
}

export async function memoryListNamespaces(): Promise<string[]> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  const resp = await callCoreRpc<{ data?: { namespaces?: string[] }; namespaces?: string[] }>({
    method: 'openhuman.memory_list_namespaces',
  });
  if (resp && typeof resp === 'object') {
    if (Array.isArray(resp)) return resp;
    const ns = resp.data?.namespaces ?? resp.namespaces;
    if (Array.isArray(ns)) return ns;
  }
  return [];
}

export async function memoryDeleteDocument(
  documentId: string,
  namespace: string
): Promise<unknown> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<unknown>({
    method: 'openhuman.memory_delete_document',
    params: { document_id: documentId, namespace },
  });
}

export async function memoryClearNamespace(
  namespace: string
): Promise<{ cleared: boolean; namespace: string }> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  const response = await callCoreRpc<{ result: { cleared: boolean; namespace: string } }>({
    method: 'openhuman.memory_clear_namespace',
    params: { namespace },
  });
  return response.result;
}

export async function memoryQueryNamespace(
  namespace: string,
  query: string,
  maxChunks?: number
): Promise<MemoryQueryResult> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  const resp = await callCoreRpc<unknown>({
    method: 'openhuman.memory_query_namespace',
    params: { namespace, query, max_chunks: maxChunks },
  });
  return unwrapMemoryQueryResult(resp);
}

export async function memoryRecallNamespace(
  namespace: string,
  maxChunks?: number
): Promise<MemoryQueryResult> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  const resp = await callCoreRpc<unknown>({
    method: 'openhuman.memory_recall_context',
    params: { namespace, max_chunks: maxChunks },
  });
  return unwrapMemoryQueryResult(resp);
}

export async function memoryGraphQuery(
  namespace?: string,
  subject?: string,
  predicate?: string
): Promise<GraphRelation[]> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  const raw = await callCoreRpc<GraphRelation[] | { result: GraphRelation[] }>({
    method: 'openhuman.memory_graph_query',
    params: { namespace, subject, predicate },
  });
  // RpcOutcome wraps with { result, logs } when logs are present — unwrap if needed.
  if (Array.isArray(raw)) return raw;
  if (raw && typeof raw === 'object' && 'result' in raw && Array.isArray(raw.result))
    return raw.result;
  console.debug(
    '[memoryGraphQuery] unexpected response shape, returning empty array. Raw response:',
    raw
  );
  return [];
}

export async function memoryDocIngest(params: {
  namespace: string;
  key: string;
  title: string;
  content: string;
  source_type?: string;
  priority?: string;
  tags?: string[];
  metadata?: Record<string, unknown>;
  category?: string;
  session_id?: string;
  document_id?: string;
}): Promise<unknown> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<unknown>({ method: 'openhuman.memory_doc_ingest', params });
}

export async function aiListMemoryFiles(relativeDir = 'memory'): Promise<string[]> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  const resp = await callCoreRpc<{ data?: { files?: string[] }; files?: string[] }>({
    method: 'openhuman.memory_list_files',
    params: { relative_dir: relativeDir },
  });
  // Unwrap envelope: registry returns { data: { files: [...] } }
  if (resp && typeof resp === 'object') {
    if (Array.isArray(resp)) return resp;
    const files = resp.data?.files ?? resp.files;
    if (Array.isArray(files)) return files;
  }
  return [];
}

export async function aiReadMemoryFile(relativePath: string): Promise<string> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  const resp = await callCoreRpc<{ data?: { content?: string }; content?: string } | string>({
    method: 'openhuman.memory_read_file',
    params: { relative_path: relativePath },
  });
  if (typeof resp === 'string') return resp;
  if (resp && typeof resp === 'object') {
    return resp.data?.content ?? resp.content ?? '';
  }
  return '';
}

export async function aiWriteMemoryFile(relativePath: string, content: string): Promise<void> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  await callCoreRpc<boolean>({
    method: 'openhuman.memory_write_file',
    params: { relative_path: relativePath, content },
  });
}

export interface MemorySyncChannelResult {
  requested: boolean;
  channel_id: string;
}

export interface MemorySyncAllResult {
  requested: boolean;
}

export interface NamespaceLearnResult {
  namespace: string;
  status: 'ok' | 'skipped' | 'error';
  error?: string;
}

export interface MemoryLearnAllResult {
  namespaces_processed: number;
  results: NamespaceLearnResult[];
}

/**
 * Request a memory sync for a specific channel.
 * Publishes MemorySyncRequested on the core event bus and returns confirmation.
 * No ingestion runs synchronously — future subscribers will react.
 */
export async function memorySyncChannel(channelId: string): Promise<MemorySyncChannelResult> {
  console.debug('[memory.sync] memorySyncChannel: entry channel_id=%s', channelId);
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  const resp = await callCoreRpc<MemorySyncChannelResult>({
    method: 'openhuman.memory_sync_channel',
    params: { channel_id: channelId },
  });
  console.debug('[memory.sync] memorySyncChannel: exit result=%o', resp);
  return resp;
}

/**
 * Request a memory sync for all channels.
 * Publishes MemorySyncRequested { channel_id: None } on the core event bus.
 */
export async function memorySyncAll(): Promise<MemorySyncAllResult> {
  console.debug('[memory.sync] memorySyncAll: entry');
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  const resp = await callCoreRpc<MemorySyncAllResult>({ method: 'openhuman.memory_sync_all' });
  console.debug('[memory.sync] memorySyncAll: exit result=%o', resp);
  return resp;
}

/**
 * Run the tree summarizer over all memory namespaces (or a subset).
 * Processes sequentially; a failing namespace is recorded, not fatal.
 */
export async function memoryLearnAll(namespaces?: string[]): Promise<MemoryLearnAllResult> {
  console.debug('[memory.learn] memoryLearnAll: entry namespaces=%o', namespaces);
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  const params: Record<string, unknown> = {};
  if (namespaces && namespaces.length > 0) {
    params.namespaces = namespaces;
  }
  const resp = await callCoreRpc<MemoryLearnAllResult>({
    method: 'openhuman.memory_learn_all',
    params,
  });
  console.debug('[memory.learn] memoryLearnAll: exit processed=%d', resp?.namespaces_processed);
  return resp;
}
