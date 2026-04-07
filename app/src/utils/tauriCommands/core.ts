/**
 * Core process and update commands.
 */
import { invoke } from '@tauri-apps/api/core';

import { callCoreRpc } from '../../services/coreRpcClient';
import { CommandResponse, isTauri } from './common';

export interface CoreUpdateStatus {
  running_version: string;
  minimum_version: string;
  /** True if running < minimum (compatibility issue). */
  outdated: boolean;
  /** Latest version on GitHub Releases (if fetch succeeded). */
  latest_version: string | null;
  /** True if running < latest (newer release available). */
  update_available: boolean;
}

export type DoctorSeverity = 'Ok' | 'Warn' | 'Error';
export type ModelProbeOutcome = 'Ok' | 'Skipped' | 'AuthOrAccess' | 'Error';

export interface DoctorReport {
  items: { severity: DoctorSeverity; category: string; message: string }[];
  summary: { ok: number; warnings: number; errors: number };
}

export interface ModelProbeReport {
  entries: { provider: string; outcome: ModelProbeOutcome; message?: string | null }[];
  summary: { ok: number; skipped: number; auth_or_access: number; errors: number };
}

export interface MigrationStats {
  from_sqlite: number;
  from_markdown: number;
  imported: number;
  skipped_unchanged: number;
  renamed_conflicts: number;
}

export interface MigrationReport {
  source_workspace: string;
  target_workspace: string;
  dry_run: boolean;
  stats: MigrationStats;
  warnings: string[];
}

/**
 * Restart the core sidecar process.
 */
export async function restartCoreProcess(): Promise<void> {
  if (!isTauri()) {
    console.debug('[core] restartCoreProcess: skipped — not running in Tauri');
    return;
  }
  console.debug('[core] restartCoreProcess: invoking restart_core_process');
  await invoke<void>('restart_core_process');
  console.debug('[core] restartCoreProcess: done');
}

/**
 * Check if the running core sidecar is outdated compared to what the app expects.
 */
export const checkCoreUpdate = async (): Promise<CoreUpdateStatus | null> => {
  if (!isTauri()) {
    console.debug('[core-update] checkCoreUpdate: skipped — not running in Tauri');
    return null;
  }
  console.debug('[core-update] checkCoreUpdate: invoking check_core_update');
  const result = await invoke<CoreUpdateStatus>('check_core_update');
  console.debug('[core-update] checkCoreUpdate: result', result);
  return result;
};

/**
 * Trigger a full core update.
 */
export const applyCoreUpdate = async (): Promise<void> => {
  if (!isTauri()) {
    console.debug('[core-update] applyCoreUpdate: skipped — not running in Tauri');
    return;
  }
  console.debug('[core-update] applyCoreUpdate: invoking apply_core_update');
  await invoke<void>('apply_core_update');
  console.debug('[core-update] applyCoreUpdate: done');
};

export async function resetOpenHumanDataAndRestartCore(): Promise<void> {
  if (!isTauri()) {
    console.debug('[core] resetOpenHumanDataAndRestartCore: skipped — not running in Tauri');
    return;
  }
  console.debug(
    '[core] resetOpenHumanDataAndRestartCore: invoking openhuman.config_reset_local_data'
  );
  await callCoreRpc({ method: 'openhuman.config_reset_local_data' });
  console.debug(
    '[core] resetOpenHumanDataAndRestartCore: local data reset complete, restarting core'
  );
  await restartCoreProcess();
  console.debug('[core] resetOpenHumanDataAndRestartCore: done');
}

/** Read onboarding_completed from core config. */
export async function getOnboardingCompleted(): Promise<boolean> {
  if (!isTauri()) return false;
  const res = await callCoreRpc<boolean | { result: boolean }>({
    method: 'openhuman.config_get_onboarding_completed',
  });
  // RpcOutcome may wrap value in { result, logs } when logs are present
  if (typeof res === 'boolean') return res;
  if (res && typeof res === 'object' && 'result' in res) return res.result;
  return false;
}

/** Write onboarding_completed to core config. */
export async function setOnboardingCompleted(value: boolean): Promise<boolean> {
  if (!isTauri()) return false;
  const res = await callCoreRpc<boolean | { result: boolean }>({
    method: 'openhuman.config_set_onboarding_completed',
    params: { value },
  });
  if (typeof res === 'boolean') return res;
  if (res && typeof res === 'object' && 'result' in res) return res.result;
  return false;
}

export async function openhumanDoctorReport(): Promise<CommandResponse<DoctorReport>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<DoctorReport>>({ method: 'openhuman.doctor_report' });
}

export async function openhumanDoctorModels(
  useCache = true
): Promise<CommandResponse<ModelProbeReport>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ModelProbeReport>>({
    method: 'openhuman.doctor_models',
    params: { use_cache: useCache },
  });
}

export async function openhumanMigrateOpenclaw(
  sourceWorkspace?: string,
  dryRun = true
): Promise<CommandResponse<MigrationReport>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<MigrationReport>>({
    method: 'openhuman.migrate_openclaw',
    params: { source_workspace: sourceWorkspace, dry_run: dryRun },
  });
}
