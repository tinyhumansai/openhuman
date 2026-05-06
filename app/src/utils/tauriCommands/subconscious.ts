/**
 * Subconscious engine commands — task management, escalations, execution log.
 */
import { callCoreRpc } from '../../services/coreRpcClient';
import { type CommandResponse, isTauri } from './common';

// ── Types ────────────────────────────────────────────────────────────────────

export interface SubconsciousTask {
  id: string;
  title: string;
  source: 'system' | 'user';
  recurrence: string;
  enabled: boolean;
  last_run_at: number | null;
  next_run_at: number | null;
  completed: boolean;
  created_at: number;
}

export interface SubconsciousLogEntry {
  id: string;
  task_id: string;
  tick_at: number;
  decision: 'noop' | 'act' | 'escalate' | 'dismissed' | string;
  result: string | null;
  duration_ms: number | null;
  created_at: number;
}

export interface SubconsciousEscalation {
  id: string;
  task_id: string;
  log_id: string | null;
  title: string;
  description: string;
  priority: 'critical' | 'important' | 'normal';
  status: 'pending' | 'approved' | 'dismissed';
  created_at: number;
  resolved_at: number | null;
}

export interface SubconsciousStatus {
  enabled: boolean;
  interval_minutes: number;
  last_tick_at: number | null;
  total_ticks: number;
  task_count: number;
  pending_escalations: number;
  consecutive_failures: number;
}

export interface TickResult {
  tick_at: number;
  evaluations: Array<{ task_id: string; decision: string; reason: string }>;
  executed: number;
  escalated: number;
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

// ── Tasks CRUD ───────────────────────────────────────────────────────────────

export async function subconsciousTasksList(
  enabledOnly = false
): Promise<CommandResponse<SubconsciousTask[]>> {
  if (!isTauri()) throw new Error('Not running in Tauri');
  return await callCoreRpc<CommandResponse<SubconsciousTask[]>>({
    method: 'openhuman.subconscious_tasks_list',
    params: { enabled_only: enabledOnly },
  });
}

export async function subconsciousTasksAdd(
  title: string,
  source: 'user' | 'system' = 'user'
): Promise<CommandResponse<SubconsciousTask>> {
  if (!isTauri()) throw new Error('Not running in Tauri');
  return await callCoreRpc<CommandResponse<SubconsciousTask>>({
    method: 'openhuman.subconscious_tasks_add',
    params: { title, source },
  });
}

export async function subconsciousTasksUpdate(
  taskId: string,
  patch: { title?: string; enabled?: boolean }
): Promise<CommandResponse<{ updated: string }>> {
  if (!isTauri()) throw new Error('Not running in Tauri');
  return await callCoreRpc<CommandResponse<{ updated: string }>>({
    method: 'openhuman.subconscious_tasks_update',
    params: { task_id: taskId, ...patch },
  });
}

export async function subconsciousTasksRemove(
  taskId: string
): Promise<CommandResponse<{ removed: string }>> {
  if (!isTauri()) throw new Error('Not running in Tauri');
  return await callCoreRpc<CommandResponse<{ removed: string }>>({
    method: 'openhuman.subconscious_tasks_remove',
    params: { task_id: taskId },
  });
}

// ── Log ──────────────────────────────────────────────────────────────────────

export async function subconsciousLogList(
  taskId?: string,
  limit = 50
): Promise<CommandResponse<SubconsciousLogEntry[]>> {
  if (!isTauri()) throw new Error('Not running in Tauri');
  return await callCoreRpc<CommandResponse<SubconsciousLogEntry[]>>({
    method: 'openhuman.subconscious_log_list',
    params: { task_id: taskId, limit },
  });
}

// ── Escalations ──────────────────────────────────────────────────────────────

export async function subconsciousEscalationsList(
  status?: 'pending' | 'approved' | 'dismissed'
): Promise<CommandResponse<SubconsciousEscalation[]>> {
  if (!isTauri()) throw new Error('Not running in Tauri');
  return await callCoreRpc<CommandResponse<SubconsciousEscalation[]>>({
    method: 'openhuman.subconscious_escalations_list',
    params: status ? { status } : {},
  });
}

export async function subconsciousEscalationsApprove(
  escalationId: string
): Promise<CommandResponse<{ approved: string }>> {
  if (!isTauri()) throw new Error('Not running in Tauri');
  return await callCoreRpc<CommandResponse<{ approved: string }>>({
    method: 'openhuman.subconscious_escalations_approve',
    params: { escalation_id: escalationId },
  });
}

export async function subconsciousEscalationsDismiss(
  escalationId: string
): Promise<CommandResponse<{ dismissed: string }>> {
  if (!isTauri()) throw new Error('Not running in Tauri');
  return await callCoreRpc<CommandResponse<{ dismissed: string }>>({
    method: 'openhuman.subconscious_escalations_dismiss',
    params: { escalation_id: escalationId },
  });
}

// ── #623: proactive reflection layer ─────────────────────────────────────────

/**
 * Categorisation of the underlying signal that produced the reflection.
 * Mirrors `subconscious::reflection::ReflectionKind` on the Rust side.
 */
export type ReflectionKind =
  | 'hotness_spike'
  | 'cross_source_pattern'
  | 'daily_digest'
  | 'due_item'
  | 'risk'
  | 'opportunity';

/**
 * Whether the reflection was surfaced into the proactive conversation
 * (`notify`) or persisted only as a quiet observation (`observe`).
 */
export type ReflectionDisposition = 'observe' | 'notify';

/**
 * One persisted observation about the user's state. Created by the
 * subconscious tick LLM.
 */
export interface Reflection {
  id: string;
  kind: ReflectionKind;
  body: string;
  disposition: ReflectionDisposition;
  proposed_action: string | null;
  source_refs: string[];
  created_at: number;
  surfaced_at: number | null;
  acted_on_at: number | null;
  dismissed_at: number | null;
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

/**
 * Drive `start_chat` against the user's active orchestrator thread with
 * a primer composed from the reflection's body and proposed_action.
 * Marks `acted_on_at`. The `targetThreadId` is the user's normal chat
 * thread, NOT the subconscious feed — taps move the conversation into
 * the user's chat surface.
 */
export async function actOnReflection(
  reflectionId: string,
  targetThreadId: string
): Promise<CommandResponse<{ request_id: string; reflection_id: string }>> {
  if (!isTauri()) throw new Error('Not running in Tauri');
  return await callCoreRpc<CommandResponse<{ request_id: string; reflection_id: string }>>({
    method: 'openhuman.subconscious_reflections_act',
    params: { reflection_id: reflectionId, target_thread_id: targetThreadId },
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
