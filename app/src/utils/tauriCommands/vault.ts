/**
 * Vault (knowledge vault) commands — folder-of-files ingested into memory.
 */
import { callCoreRpc } from '../../services/coreRpcClient';
import { CommandResponse, isTauri } from './common';

export interface CoreVault {
  id: string;
  name: string;
  root_path: string;
  host_os?: string | null;
  namespace: string;
  include_globs: string[];
  exclude_globs: string[];
  created_at: string;
  last_synced_at?: string | null;
  file_count: number;
  write_state: CoreVaultWriteState;
  write_state_reason?: string | null;
}

export type CoreVaultWriteState = 'writable' | 'read_only' | 'unavailable';

export type CoreVaultFileStatus = 'ok' | 'skipped' | 'failed';

export interface CoreVaultFile {
  vault_id: string;
  rel_path: string;
  document_id: string;
  content_hash: string;
  mtime_ms: number;
  bytes: number;
  ingested_at: string;
  status: CoreVaultFileStatus;
}

export type CoreVaultSyncStatus = 'idle' | 'running' | 'completed' | 'failed';

/** Live progress returned by `openhuman.vault_sync_status`. */
export interface CoreVaultSyncState {
  vault_id: string;
  status: CoreVaultSyncStatus;
  scanned: number;
  ingested: number;
  unchanged: number;
  removed: number;
  failed: number;
  skipped_unsupported: number;
  /** Total files queued for ingestion; 0 while the discovery walk is still running. */
  total: number;
  started_at_ms: number;
  finished_at_ms: number | null;
  /** Wall-clock ms; 0 while running; set on completion. */
  duration_ms: number;
  errors: string[];
}

function ensureTauri() {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
}

export async function openhumanVaultList(): Promise<CommandResponse<CoreVault[]>> {
  ensureTauri();
  return await callCoreRpc<CommandResponse<CoreVault[]>>({ method: 'openhuman.vault_list' });
}

export async function openhumanVaultCreate(params: {
  name: string;
  rootPath: string;
  includeGlobs?: string[];
  excludeGlobs?: string[];
}): Promise<CommandResponse<CoreVault>> {
  ensureTauri();
  return await callCoreRpc<CommandResponse<CoreVault>>({
    method: 'openhuman.vault_create',
    params: {
      name: params.name,
      root_path: params.rootPath,
      include_globs: params.includeGlobs ?? [],
      exclude_globs: params.excludeGlobs ?? [],
    },
  });
}

export async function openhumanVaultGet(vaultId: string): Promise<CommandResponse<CoreVault>> {
  ensureTauri();
  return await callCoreRpc<CommandResponse<CoreVault>>({
    method: 'openhuman.vault_get',
    params: { vault_id: vaultId },
  });
}

export async function openhumanVaultFiles(
  vaultId: string
): Promise<CommandResponse<CoreVaultFile[]>> {
  ensureTauri();
  return await callCoreRpc<CommandResponse<CoreVaultFile[]>>({
    method: 'openhuman.vault_files',
    params: { vault_id: vaultId },
  });
}

export async function openhumanVaultWriteMarkdown(params: {
  vaultId: string;
  relPath: string;
  content: string;
  overwrite?: boolean;
  approved: boolean;
}): Promise<
  CommandResponse<{ vault_id: string; rel_path: string; bytes_written: number; created: boolean }>
> {
  ensureTauri();
  return await callCoreRpc<
    CommandResponse<{ vault_id: string; rel_path: string; bytes_written: number; created: boolean }>
  >({
    method: 'openhuman.vault_write_markdown',
    params: {
      vault_id: params.vaultId,
      rel_path: params.relPath,
      content: params.content,
      overwrite: params.overwrite ?? false,
      approved: params.approved,
    },
  });
}

export async function openhumanVaultRemove(
  vaultId: string,
  purgeMemory: boolean
): Promise<CommandResponse<{ vault_id: string; removed: boolean; purged: boolean }>> {
  ensureTauri();
  return await callCoreRpc<
    CommandResponse<{ vault_id: string; removed: boolean; purged: boolean }>
  >({ method: 'openhuman.vault_remove', params: { vault_id: vaultId, purge_memory: purgeMemory } });
}

export async function openhumanVaultSync(
  vaultId: string
): Promise<CommandResponse<{ status: string; vault_id: string }>> {
  ensureTauri();
  return await callCoreRpc<CommandResponse<{ status: string; vault_id: string }>>({
    method: 'openhuman.vault_sync',
    params: { vault_id: vaultId },
  });
}

/** Poll live sync progress for a vault. */
export async function openhumanVaultSyncStatus(
  vaultId: string
): Promise<CommandResponse<CoreVaultSyncState>> {
  ensureTauri();
  return await callCoreRpc<CommandResponse<CoreVaultSyncState>>({
    method: 'openhuman.vault_sync_status',
    params: { vault_id: vaultId },
  });
}
