import { callCoreRpc } from '../../services/coreRpcClient';
import type {
  CanvasTask,
  CanvasTrackerSettings,
  LocalStatus,
  ReminderRecommendation,
  SyncSummary,
} from './types';

function unwrapCliEnvelope<T>(value: unknown): T {
  if (
    value !== null &&
    typeof value === 'object' &&
    'result' in (value as Record<string, unknown>) &&
    'logs' in (value as Record<string, unknown>) &&
    Array.isArray((value as { logs: unknown }).logs)
  ) {
    return (value as { result: T }).result;
  }
  return value as T;
}

export async function getCanvasTrackerSettings(): Promise<CanvasTrackerSettings> {
  const raw = await callCoreRpc<unknown>({ method: 'openhuman.canvas_tracker_get_settings' });
  return unwrapCliEnvelope<CanvasTrackerSettings>(raw);
}

export async function updateCanvasTrackerSettings(input: {
  settings: CanvasTrackerSettings;
  token?: string;
  clear_token?: boolean;
}): Promise<CanvasTrackerSettings> {
  const raw = await callCoreRpc<unknown>({
    method: 'openhuman.canvas_tracker_update_settings',
    params: input,
  });
  return unwrapCliEnvelope<CanvasTrackerSettings>(raw);
}

export async function syncCanvasTrackerNow(): Promise<SyncSummary> {
  const raw = await callCoreRpc<unknown>({ method: 'openhuman.canvas_tracker_sync_now' });
  return unwrapCliEnvelope<SyncSummary>(raw);
}

export async function listCanvasTrackerTasks(): Promise<CanvasTask[]> {
  const raw = await callCoreRpc<unknown>({ method: 'openhuman.canvas_tracker_list_tasks' });
  return unwrapCliEnvelope<CanvasTask[]>(raw);
}

export async function updateCanvasTaskStatus(input: {
  course_id: string;
  assignment_id: string;
  status: LocalStatus;
}): Promise<{ updated: boolean }> {
  const raw = await callCoreRpc<unknown>({
    method: 'openhuman.canvas_tracker_update_local_status',
    params: input,
  });
  return unwrapCliEnvelope<{ updated: boolean }>(raw);
}

export async function listCanvasTrackerReminders(): Promise<ReminderRecommendation[]> {
  const raw = await callCoreRpc<unknown>({ method: 'openhuman.canvas_tracker_list_reminders' });
  return unwrapCliEnvelope<ReminderRecommendation[]>(raw);
}
