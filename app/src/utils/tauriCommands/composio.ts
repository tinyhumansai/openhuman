import { callCoreRpc } from '../../services/coreRpcClient';
import { type CommandResponse, isTauri } from './common';

export interface ComposioTriggerHistoryEntry {
  received_at_ms: number;
  toolkit: string;
  trigger: string;
  metadata_id: string;
  metadata_uuid: string;
  payload: unknown;
}

export interface ComposioTriggerHistoryResult {
  archive_dir: string;
  current_day_file: string;
  entries: ComposioTriggerHistoryEntry[];
}

export async function openhumanComposioListTriggerHistory(
  limit = 100
): Promise<CommandResponse<{ result: ComposioTriggerHistoryResult }>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }

  return await callCoreRpc<CommandResponse<{ result: ComposioTriggerHistoryResult }>>({
    method: 'openhuman.composio_list_trigger_history',
    params: { limit },
  });
}

// ── Direct mode (BYO API key) ───────────────────────────────────────
//
// [composio-direct] These three RPCs back the Settings > Composio panel
// added in PR3 of #1710. The API key itself is *never* read back over
// RPC — only a boolean flag (`api_key_set`) so the UI can show a
// "Key stored ✓" / "No key set" status without exposing the secret.

export interface ComposioModeStatus {
  mode: 'backend' | 'direct' | string;
  api_key_set: boolean;
}

/// Read the current Composio routing mode and whether a direct-mode API
/// key is stored. The key itself is never returned.
export async function openhumanComposioGetMode(): Promise<CommandResponse<ComposioModeStatus>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ComposioModeStatus>>({
    method: 'openhuman.composio_get_mode',
  });
}

export interface ComposioSetApiKeyResult {
  stored: boolean;
  mode: string;
}

/// Persist a Composio API key for direct mode and (optionally) flip the
/// routing mode to "direct".
export async function openhumanComposioSetApiKey(
  apiKey: string,
  activateDirect = true
): Promise<CommandResponse<ComposioSetApiKeyResult>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ComposioSetApiKeyResult>>({
    method: 'openhuman.composio_set_api_key',
    params: { api_key: apiKey, activate_direct: activateDirect },
  });
}

/// Remove the stored direct-mode API key and reset the routing mode to
/// "backend".
export async function openhumanComposioClearApiKey(): Promise<
  CommandResponse<{ cleared: boolean; mode: string }>
> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<{ cleared: boolean; mode: string }>>({
    method: 'openhuman.composio_clear_api_key',
  });
}
