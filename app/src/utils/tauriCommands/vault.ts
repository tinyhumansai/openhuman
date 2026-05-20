/**
 * Vault (knowledge vault) commands — folder-of-files ingested into memory.
 */
import { callCoreRpc } from '../../services/coreRpcClient';
import { CommandResponse, isTauri } from './common';

export interface CoreVault {
  id: string;
  name: string;
  root_path: string;
  namespace: string;
  include_globs: string[];
  exclude_globs: string[];
  created_at: string;
  last_synced_at?: string | null;
  file_count: number;
}

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

export interface CoreVaultSyncReport {
  vault_id: string;
  scanned: number;
  ingested: number;
  unchanged: number;
  removed: number;
  failed: number;
  skipped_unsupported: number;
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
): Promise<CommandResponse<CoreVaultSyncReport>> {
  ensureTauri();
  return await callCoreRpc<CommandResponse<CoreVaultSyncReport>>({
    method: 'openhuman.vault_sync',
    params: { vault_id: vaultId },
  });
}
