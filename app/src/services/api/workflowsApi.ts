import debug from 'debug';

import { callCoreRpc } from '../coreRpcClient';

const log = debug('workflowsApi');

/**
 * Tool allow/deny scope for a workflow or phase.
 * Mirrors `openhuman::workflows::types::ToolScope`.
 */
export interface ToolScope {
  allow: string[];
  deny: string[];
}

/**
 * A single workflow phase definition.
 * Mirrors `openhuman::workflows::types::WorkflowPhase`.
 */
export interface WorkflowPhase {
  description?: string | null;
  rules: string[];
  scripts: string[];
  tools?: ToolScope | null;
  context: string[];
}

/**
 * Summary of a workflow returned by `openhuman.workflows_list`.
 * Mirrors `openhuman::workflows::types::WorkflowSummary`.
 */
export interface WorkflowSummary {
  /** Stable identifier — usually the slugified directory name. */
  id: string;
  /** Display name from frontmatter. */
  name: string;
  /** Short description from frontmatter. */
  description: string;
  /** When the agent should pick up this workflow. */
  when_to_use: string;
  /** Tags declared in frontmatter. */
  tags: string[];
  /** Where the workflow came from: user-scope or project-scope. */
  scope: 'user' | 'project';
  /** Phase names declared in this workflow. */
  phases: string[];
  /** Non-fatal parse warnings to surface in the UI. */
  warnings: string[];
}

/**
 * Full workflow definition returned by `openhuman.workflows_read`.
 * Mirrors `openhuman::workflows::types::Workflow`.
 */
export interface Workflow {
  name: string;
  dir_name: string;
  description: string;
  when_to_use: string;
  tags: string[];
  tools?: ToolScope | null;
  phases: Record<string, WorkflowPhase>;
  location?: string | null;
  scope: 'user' | 'project';
  warnings: string[];
}

interface WorkflowsListResult {
  workflows: WorkflowSummary[];
}

interface WorkflowsReadResult {
  workflow: Workflow;
}

interface WorkflowsCreateResult {
  workflow: Workflow;
}

interface WorkflowsUninstallResult {
  id: string;
  removed: boolean;
}

/**
 * Phase guidance returned by `openhuman.workflows_phase`.
 */
export interface WorkflowPhaseResult {
  guidance: string | null;
  tool_scope: ToolScope | null;
}

interface Envelope<T> {
  data?: T;
}

function unwrapEnvelope<T>(response: Envelope<T> | T): T {
  if (response && typeof response === 'object' && 'data' in response) {
    const envelope = response as Envelope<T>;
    if (envelope.data !== undefined) {
      return envelope.data as T;
    }
  }
  return response as T;
}

export const workflowsApi = {
  /**
   * Enumerate workflows visible in the active workspace via
   * `openhuman.workflows_list`.
   */
  listWorkflows: async (): Promise<WorkflowSummary[]> => {
    log('listWorkflows: request');
    const response = await callCoreRpc<Envelope<WorkflowsListResult> | WorkflowsListResult>({
      method: 'openhuman.workflows_list',
    });
    const result = unwrapEnvelope(response);
    const workflows = result?.workflows ?? [];
    log('listWorkflows: response count=%d', workflows.length);
    return workflows;
  },

  /**
   * Read a single workflow by id via `openhuman.workflows_read`.
   */
  readWorkflow: async (id: string): Promise<Workflow> => {
    log('readWorkflow: request id=%s', id);
    const response = await callCoreRpc<Envelope<WorkflowsReadResult> | WorkflowsReadResult>({
      method: 'openhuman.workflows_read',
      params: { id },
    });
    const result = unwrapEnvelope(response);
    log('readWorkflow: response name=%s', result.workflow.name);
    return result.workflow;
  },

  /**
   * Create a new workflow via `openhuman.workflows_create`.
   *
   * The Rust side slugifies the name, writes the workflow directory with the
   * supplied metadata, and returns the freshly-created Workflow so the caller
   * can update local state without a full refetch.
   */
  createWorkflow: async (params: {
    name: string;
    description?: string;
    when_to_use?: string;
  }): Promise<Workflow> => {
    log('createWorkflow: request name=%s', params.name);
    const response = await callCoreRpc<Envelope<WorkflowsCreateResult> | WorkflowsCreateResult>({
      method: 'openhuman.workflows_create',
      params: {
        name: params.name,
        ...(params.description !== undefined ? { description: params.description } : {}),
        ...(params.when_to_use !== undefined ? { when_to_use: params.when_to_use } : {}),
      },
    });
    const result = unwrapEnvelope(response);
    log('createWorkflow: response name=%s', result.workflow.name);
    return result.workflow;
  },

  /**
   * Uninstall a workflow by id via `openhuman.workflows_uninstall`.
   *
   * Only user-scope workflows can be uninstalled. Project-scope workflows are
   * read-only — the backend returns an error which surfaces as a rejected
   * promise.
   */
  uninstallWorkflow: async (id: string): Promise<WorkflowsUninstallResult> => {
    log('uninstallWorkflow: request id=%s', id);
    const response = await callCoreRpc<
      Envelope<WorkflowsUninstallResult> | WorkflowsUninstallResult
    >({ method: 'openhuman.workflows_uninstall', params: { id } });
    const result = unwrapEnvelope(response);
    log('uninstallWorkflow: response id=%s removed=%s', result.id, result.removed);
    return result;
  },

  /**
   * Fetch guidance and tool scope for a specific phase of a workflow via
   * `openhuman.workflows_phase`.
   */
  getWorkflowPhase: async (id: string, phase: string): Promise<WorkflowPhaseResult> => {
    log('getWorkflowPhase: request id=%s phase=%s', id, phase);
    const response = await callCoreRpc<Envelope<WorkflowPhaseResult> | WorkflowPhaseResult>({
      method: 'openhuman.workflows_phase',
      params: { id, phase },
    });
    const result = unwrapEnvelope(response);
    log('getWorkflowPhase: response guidance=%s', result.guidance != null ? 'yes' : 'null');
    return result;
  },
};
