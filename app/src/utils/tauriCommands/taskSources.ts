/**
 * Task-sources commands.
 *
 * Thin wrappers around the core `openhuman.task_sources_*` JSON-RPC
 * surface. These operations return bare values (the core ops attach no
 * log envelope), so `callCoreRpc<T>` resolves directly to the typed
 * payload.
 */
import { callCoreRpc } from '../../services/coreRpcClient';
import { isTauri } from './common';

export type TaskSourceProvider = 'github' | 'notion' | 'linear' | 'clickup';

export type TaskSourceTarget = 'agent_todo_proactive' | 'todo_only';

/** Per-provider filter, discriminated by `provider`. Mirrors the Rust
 *  `FilterSpec` (serde snake_case, tagged by `provider`). */
export type TaskSourceFilter =
  | {
      provider: 'github';
      repo?: string;
      labels?: string[];
      assignee_is_me?: boolean;
      state?: string;
      extra?: Record<string, unknown>;
    }
  | {
      provider: 'notion';
      database_id?: string;
      assigned_to_me?: boolean;
      status?: string;
      extra?: Record<string, unknown>;
    }
  | {
      provider: 'linear';
      team_id?: string;
      assignee_is_me?: boolean;
      state?: string;
      extra?: Record<string, unknown>;
    }
  | {
      provider: 'clickup';
      team_id?: string;
      list_id?: string;
      assignee_is_me?: boolean;
      extra?: Record<string, unknown>;
    };

export interface TaskSource {
  id: string;
  provider: TaskSourceProvider;
  connectionId?: string;
  name?: string;
  enabled: boolean;
  filter: TaskSourceFilter;
  intervalSecs: number;
  target: TaskSourceTarget;
  maxTasksPerFetch: number;
  /** Static executor routing (G7): personality/skill/agent handle every card
   *  from this source is pre-assigned to. */
  assignedExecutor?: string;
  createdAt: string;
  lastFetchAt?: string;
  lastStatus?: string;
}

export interface NormalizedTask {
  externalId: string;
  sourceId: string;
  provider: string;
  title: string;
  body?: string;
  url?: string;
  status?: string;
  assignee?: string;
  due?: string;
  labels: string[];
  priority?: string;
  updatedAt?: string;
}

export interface FetchOutcome {
  sourceId: string;
  provider: string;
  fetched: number;
  routed: number;
  skippedDupe: number;
  error?: string;
}

export interface TaskSourcesStatus {
  enabled: boolean;
  defaultIntervalSecs: number;
  sourceCount: number;
  enabledSourceCount: number;
}

export interface TaskSourcePatch {
  name?: string;
  enabled?: boolean;
  filter?: TaskSourceFilter;
  intervalSecs?: number;
  target?: TaskSourceTarget;
  maxTasksPerFetch?: number;
  connectionId?: string;
  /** Executor routing (G7): personality/skill/agent handle to pre-assign. */
  assignedExecutor?: string;
}

export interface TaskSourceAddParams {
  provider: TaskSourceProvider;
  filter: TaskSourceFilter;
  name?: string;
  connection_id?: string;
  interval_secs?: number;
  target?: TaskSourceTarget;
  max_tasks_per_fetch?: number;
  assigned_executor?: string;
}

function ensureTauri(): void {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
}

export async function openhumanTaskSourcesList(): Promise<TaskSource[]> {
  ensureTauri();
  return await callCoreRpc<TaskSource[]>({ method: 'openhuman.task_sources_list' });
}

export async function openhumanTaskSourcesGet(id: string): Promise<TaskSource> {
  ensureTauri();
  return await callCoreRpc<TaskSource>({ method: 'openhuman.task_sources_get', params: { id } });
}

export async function openhumanTaskSourcesAdd(params: TaskSourceAddParams): Promise<TaskSource> {
  ensureTauri();
  return await callCoreRpc<TaskSource>({
    method: 'openhuman.task_sources_add',
    params: params as unknown as Record<string, unknown>,
  });
}

export async function openhumanTaskSourcesUpdate(
  id: string,
  patch: TaskSourcePatch
): Promise<TaskSource> {
  ensureTauri();
  return await callCoreRpc<TaskSource>({
    method: 'openhuman.task_sources_update',
    params: { id, patch },
  });
}

export async function openhumanTaskSourcesRemove(
  id: string
): Promise<{ id: string; removed: boolean }> {
  ensureTauri();
  return await callCoreRpc<{ id: string; removed: boolean }>({
    method: 'openhuman.task_sources_remove',
    params: { id },
  });
}

export async function openhumanTaskSourcesFetch(id: string): Promise<FetchOutcome> {
  ensureTauri();
  return await callCoreRpc<FetchOutcome>({
    method: 'openhuman.task_sources_fetch',
    params: { id },
  });
}

export async function openhumanTaskSourcesListTasks(
  id: string,
  limit = 50
): Promise<NormalizedTask[]> {
  ensureTauri();
  return await callCoreRpc<NormalizedTask[]>({
    method: 'openhuman.task_sources_list_tasks',
    params: { id, limit },
  });
}

export async function openhumanTaskSourcesPreviewFilter(
  provider: TaskSourceProvider,
  filter: TaskSourceFilter,
  connectionId?: string,
  max?: number
): Promise<NormalizedTask[]> {
  ensureTauri();
  return await callCoreRpc<NormalizedTask[]>({
    method: 'openhuman.task_sources_preview_filter',
    params: { provider, filter, connection_id: connectionId, max },
  });
}

export async function openhumanTaskSourcesStatus(): Promise<TaskSourcesStatus> {
  ensureTauri();
  return await callCoreRpc<TaskSourcesStatus>({ method: 'openhuman.task_sources_status' });
}
