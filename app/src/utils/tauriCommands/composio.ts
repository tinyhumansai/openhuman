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
