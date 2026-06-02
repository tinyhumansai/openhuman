/**
 * Subconscious engine commands — thoughts (reflections) and engine control.
 */
import { callCoreRpc } from '../../services/coreRpcClient';
import { type CommandResponse, isTauri } from './common';

// ── Types ────────────────────────────────────────────────────────────────────

export interface SubconsciousStatus {
  enabled: boolean;
  mode: 'off' | 'simple' | 'aggressive';
  provider_available: boolean;
  provider_unavailable_reason: string | null;
  interval_minutes: number;
  last_tick_at: number | null;
  total_ticks: number;
  consecutive_failures: number;
}

export interface TickResult {
  tick_at: number;
  thoughts_count: number;
  thread_id: string | null;
  duration_ms: number;
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

// ── Thoughts (Reflections) ──────────────────────────────────────────────────

export type ReflectionKind =
  | 'hotness_spike'
  | 'cross_source_pattern'
  | 'daily_digest'
  | 'due_item'
  | 'risk'
  | 'opportunity';

export interface SourceChunk {
  ref_id: string;
  kind: string;
  content: string;
  metadata?: unknown;
}

export interface Reflection {
  id: string;
  kind: ReflectionKind;
  body: string;
  proposed_action: string | null;
  source_refs: string[];
  source_chunks?: SourceChunk[];
  created_at: number;
  acted_on_at: number | null;
  dismissed_at: number | null;
  thread_id: string | null;
}

export async function listReflections(
  limit = 50,
  sinceTs?: number
): Promise<CommandResponse<Reflection[]>> {
  if (!isTauri()) throw new Error('Not running in Tauri');
  const params: Record<string, unknown> = { limit };
  if (sinceTs !== undefined) params.since_ts = sinceTs;
  return await callCoreRpc<CommandResponse<Reflection[]>>({
    method: 'openhuman.subconscious_reflections_list',
    params,
  });
}

export async function actOnReflection(
  reflectionId: string
): Promise<CommandResponse<{ reflection_id: string; thread_id: string }>> {
  if (!isTauri()) throw new Error('Not running in Tauri');
  return await callCoreRpc<CommandResponse<{ reflection_id: string; thread_id: string }>>({
    method: 'openhuman.subconscious_reflections_act',
    params: { reflection_id: reflectionId },
  });
}

export async function dismissReflection(
  reflectionId: string
): Promise<CommandResponse<{ dismissed: string }>> {
  if (!isTauri()) throw new Error('Not running in Tauri');
  return await callCoreRpc<CommandResponse<{ dismissed: string }>>({
    method: 'openhuman.subconscious_reflections_dismiss',
    params: { reflection_id: reflectionId },
  });
}
