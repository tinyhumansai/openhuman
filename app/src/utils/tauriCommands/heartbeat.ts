/**
 * Heartbeat loop commands.
 */
import { callCoreRpc } from '../../services/coreRpcClient';
import { type CommandResponse, isTauri } from './common';

export type SubconsciousMode = 'off' | 'simple' | 'aggressive' | 'event_driven';

export interface HeartbeatSettings {
  enabled: boolean;
  interval_minutes: number;
  inference_enabled: boolean;
  notify_meetings: boolean;
  notify_reminders: boolean;
  notify_relevant_events: boolean;
  external_delivery_enabled: boolean;
  meeting_lookahead_minutes: number;
  max_calendar_connections_per_tick: number;
  reminder_lookahead_minutes: number;
  subconscious_mode: SubconsciousMode;
  triggers_enabled: boolean;
  max_promotions_per_hour: number;
}

export type HeartbeatSettingsPatch = Partial<HeartbeatSettings>;

export interface HeartbeatPlannerSummary {
  source_events: number;
  deliveries_attempted: number;
  deliveries_sent: number;
  deliveries_skipped_dedup: number;
}

export async function openhumanHeartbeatSettingsGet(): Promise<
  CommandResponse<{ settings: HeartbeatSettings }>
> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<{ settings: HeartbeatSettings }>>({
    method: 'openhuman.heartbeat_settings_get',
  });
}

export async function openhumanHeartbeatSettingsSet(
  patch: HeartbeatSettingsPatch
): Promise<CommandResponse<{ settings: HeartbeatSettings }>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<{ settings: HeartbeatSettings }>>({
    method: 'openhuman.heartbeat_settings_set',
    params: patch,
  });
}

export async function openhumanHeartbeatTickNow(): Promise<
  CommandResponse<{ summary: HeartbeatPlannerSummary }>
> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<{ summary: HeartbeatPlannerSummary }>>({
    method: 'openhuman.heartbeat_tick_now',
  });
}
