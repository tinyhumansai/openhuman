/**
 * Subconscious engine commands — engine control (status / trigger).
 *
 * The subconscious now runs a structured tick (memory_diff → prepare_context
 * → decide); continuity lives in the user's global to-dos and goals rather
 * than a scratchpad, so only the status/trigger RPCs are exposed here.
 */
import { callCoreRpc } from '../../services/coreRpcClient';
import { type CommandResponse, isTauri } from './common';
import type { HeartbeatSettings } from './heartbeat';

// ── Types ────────────────────────────────────────────────────────────────────

export interface SubconsciousStatus {
  enabled: boolean;
  mode: 'off' | 'simple' | 'aggressive' | 'event_driven';
  provider_available: boolean;
  provider_unavailable_reason: string | null;
  interval_minutes: number;
  last_tick_at: number | null;
  total_ticks: number;
  consecutive_failures: number;
}

export interface TickResult {
  tick_at: number;
  duration_ms: number;
  response_chars?: number;
}

/** Status of the event-driven subconscious trigger pipeline. */
export interface SubconsciousTriggersStatus {
  triggers_enabled: boolean;
  mode: string;
  max_promotions_per_hour: number;
  orchestrator_running: boolean;
  queue_depth: number | null;
  orchestrator_thread_id: string;
  user_thread_id: string;
}

// ── Status & Trigger ─────────────────────────────────────────────────────────

export async function subconsciousStatus(): Promise<CommandResponse<SubconsciousStatus>> {
  if (!isTauri()) throw new Error('Not running in Tauri');
  return await callCoreRpc<CommandResponse<SubconsciousStatus>>({
    method: 'openhuman.subconscious_status',
  });
}

export async function subconsciousTrigger(): Promise<CommandResponse<TickResult>> {
  if (!isTauri()) throw new Error('Not running in Tauri');
  return await callCoreRpc<CommandResponse<TickResult>>({
    method: 'openhuman.subconscious_trigger',
  });
}

// The trigger-pipeline status + toggle work over any core transport (Tauri
// invoke or cloud/tunnel HTTP), so they intentionally do NOT gate on
// `isTauri()` — `callCoreRpc` resolves the active transport itself.

export async function subconsciousTriggersStatus(): Promise<
  CommandResponse<SubconsciousTriggersStatus>
> {
  return await callCoreRpc<CommandResponse<SubconsciousTriggersStatus>>({
    method: 'openhuman.subconscious_triggers_status',
  });
}

/**
 * Enable or disable the event-driven trigger pipeline.
 *
 * Enabling flips the subconscious into `event_driven` mode so the orchestrator
 * bootstraps. Disabling also resets the mode to `off` — otherwise the earlier
 * enable would leave `event_driven`/`inference_enabled` set and the legacy
 * heartbeat tick would keep running every 5 min after the pipeline is turned
 * off. The core restarts the heartbeat loop on this change.
 */
export async function setSubconsciousTriggersEnabled(
  enabled: boolean
): Promise<CommandResponse<{ settings: HeartbeatSettings }>> {
  return await callCoreRpc<CommandResponse<{ settings: HeartbeatSettings }>>({
    method: 'openhuman.heartbeat_settings_set',
    params: { triggers_enabled: enabled, subconscious_mode: enabled ? 'event_driven' : 'off' },
  });
}
