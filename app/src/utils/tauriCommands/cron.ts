/**
 * Cron job commands.
 */
import { callCoreRpc } from '../../services/coreRpcClient';
import { CommandResponse, isTauri } from './common';

export interface CoreCronScheduleCron {
  kind: 'cron';
  expr: string;
  tz?: string | null;
}

export interface CoreCronScheduleAt {
  kind: 'at';
  at: string;
}

export interface CoreCronScheduleEvery {
  kind: 'every';
  every_ms: number;
}

export type CoreCronSchedule = CoreCronScheduleCron | CoreCronScheduleAt | CoreCronScheduleEvery;

export interface CoreCronJob {
  id: string;
  expression: string;
  schedule: CoreCronSchedule;
  command: string;
  prompt?: string | null;
  name?: string | null;
  job_type: 'shell' | 'agent' | string;
  session_target: 'isolated' | 'main' | string;
  model?: string | null;
  enabled: boolean;
  delivery: { mode: string; channel?: string | null; to?: string | null; best_effort: boolean };
  delete_after_run: boolean;
  created_at: string;
  next_run: string;
  last_run?: string | null;
  last_status?: string | null;
  last_output?: string | null;
}

export interface CoreCronRun {
  id: number;
  job_id: string;
  started_at: string;
  finished_at: string;
  status: string;
  output?: string | null;
  duration_ms?: number | null;
}

export async function openhumanCronList(): Promise<CommandResponse<CoreCronJob[]>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<CoreCronJob[]>>({ method: 'openhuman.cron_list' });
}

export async function openhumanCronUpdate(
  jobId: string,
  patch: Record<string, unknown>
): Promise<CommandResponse<CoreCronJob>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<CoreCronJob>>({
    method: 'openhuman.cron_update',
    params: { job_id: jobId, patch },
  });
}

export async function openhumanCronRemove(
  jobId: string
): Promise<CommandResponse<{ job_id: string; removed: boolean }>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<{ job_id: string; removed: boolean }>>({
    method: 'openhuman.cron_remove',
    params: { job_id: jobId },
  });
}

export async function openhumanCronRun(
  jobId: string
): Promise<
  CommandResponse<{
    job_id: string;
    status: 'ok' | 'error' | string;
    duration_ms: number;
    output: string;
  }>
> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<
    CommandResponse<{
      job_id: string;
      status: 'ok' | 'error' | string;
      duration_ms: number;
      output: string;
    }>
  >({ method: 'openhuman.cron_run', params: { job_id: jobId } });
}

export async function openhumanCronRuns(
  jobId: string,
  limit = 20
): Promise<CommandResponse<CoreCronRun[]>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<CoreCronRun[]>>({
    method: 'openhuman.cron_runs',
    params: { job_id: jobId, limit },
  });
}
