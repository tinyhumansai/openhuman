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
  since_days?: number;
  max_items?: number;
  selector?: string;
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

/// i18n keys for each source kind's user-visible label. Resolve via
/// `t(SOURCE_KIND_LABEL_KEYS[kind])` in components — keeping the keys
/// as a constant lets the dialog kind-picker render the same labels
/// without each call site duplicating the switch.
export const SOURCE_KIND_LABEL_KEYS: Record<SourceKind, string> = {
  composio: 'memorySources.kind.composio',
  folder: 'memorySources.kind.folder',
  github_repo: 'memorySources.kind.github_repo',
  twitter_query: 'memorySources.kind.twitter_query',
  rss_feed: 'memorySources.kind.rss_feed',
  web_page: 'memorySources.kind.web_page',
};

export const SOURCE_KIND_ICONS: Record<SourceKind, string> = {
  composio: '🔗',
  folder: '📁',
  github_repo: '🐙',
  twitter_query: '🐦',
  rss_feed: '📡',
  web_page: '🌐',
};
