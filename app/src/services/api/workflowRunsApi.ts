/**
 * Frontend client for the declarative workflow-runs engine (#3375).
 *
 * Wraps the six `openhuman.workflow_run_*` JSON-RPC controllers exposed by the
 * Rust `workflow_runs` domain:
 *   - `workflow_run_list_definitions` — catalog of runnable workflow definitions
 *   - `workflow_run_list`             — durable runs (with filters / paging)
 *   - `workflow_run_get`              — single run (poll target for progress)
 *   - `workflow_run_start`            — start a run from a definition id
 *   - `workflow_run_stop`             — interrupt a running workflow
 *   - `workflow_run_resume`           — resume an interrupted workflow
 *
 * The Rust controllers serialize structs with `rename_all = "camelCase"` and
 * the status / safety-tier enums with `rename_all = "snake_case"`, so the wire
 * payload already matches these TypeScript shapes — no snake/camel transform is
 * needed. Progress is poll-based in this engine (no socket events yet): start a
 * run, then poll `getRun(id)` until `status` is terminal.
 */
import debug from 'debug';

import { callCoreRpc } from '../coreRpcClient';

const log = debug('workflowRunsApi');

// ---------------------------------------------------------------------------
// Wire types — mirror `src/openhuman/workflow_runs/types.rs`.
// ---------------------------------------------------------------------------

/** What a workflow's child agents are permitted to do. (`snake_case` on wire.) */
export type WorkflowSafetyTier = 'read_only' | 'standard' | 'edit_capable';

/** Lifecycle status of a durable run. (`snake_case` on wire.) */
export type WorkflowRunStatus =
  | 'pending'
  | 'running'
  | 'completed'
  | 'failed'
  | 'cancelled'
  | 'interrupted';

/** Per-phase progress status inside `WorkflowRun.phaseStates`. */
export type WorkflowPhaseStatus = 'pending' | 'running' | 'completed' | 'failed';

/** One ordered phase of a workflow definition. */
export interface WorkflowPhase {
  /** Unique within the definition (e.g. "decompose", "research"). */
  name: string;
  /** Human-readable purpose. */
  description: string;
  /** Agent definition ids fanned out (in parallel) for this phase. */
  agentIds: string[];
  /** Phase names that must complete before this one runs. */
  dependsOn: string[];
}

/** A runnable workflow definition (builtin or, later, user-authored). */
export interface WorkflowDefinition {
  id: string;
  name: string;
  description: string;
  phases: WorkflowPhase[];
  /** Max agents run concurrently within a single phase. */
  defaultConcurrency: number;
  /** Hard cap on child agents spawned across the whole run. */
  maxChildren: number;
  safetyTier: WorkflowSafetyTier;
}

/** One child agent's result within a phase. */
export interface WorkflowPhaseOutput {
  /** Orchestration id of the spawned child agent. */
  orchestrationId: string;
  /** Agent definition id (e.g. "researcher"). */
  agentId: string;
  /** The child's result summary. */
  output: string;
}

/** Progress record for a single phase, keyed by phase name in `phaseStates`. */
export interface WorkflowPhaseState {
  status: WorkflowPhaseStatus;
  outputs: WorkflowPhaseOutput[];
  /** Present only when `status === 'failed'`. */
  reason?: string;
}

/** A durable workflow run. Mirrors the Rust `WorkflowRun`. */
export interface WorkflowRun {
  id: string;
  definitionId: string;
  parentThreadId: string | null;
  /** Run input object (e.g. `{ question: "..." }`). */
  input: unknown;
  /** Per-phase progress, keyed by phase name. */
  phaseStates: Record<string, WorkflowPhaseState>;
  /** Orchestration ids of every spawned child agent. */
  childRunIds: string[];
  status: WorkflowRunStatus;
  /** Final synthesized output — present once `status === 'completed'`. */
  summary: string | null;
  startedAt: string;
  updatedAt: string;
  completedAt: string | null;
}

interface ListDefinitionsResult {
  definitions: WorkflowDefinition[];
  count: number;
}

interface ListRunsResult {
  runs: WorkflowRun[];
  count: number;
}

interface RunResult {
  workflowRun: WorkflowRun;
}

interface MaybeRunResult {
  workflowRun: WorkflowRun | null;
}

/** Filters accepted by `workflow_run_list`. */
export interface ListRunsParams {
  definitionId?: string;
  status?: WorkflowRunStatus;
  parentThreadId?: string;
  limit?: number;
  offset?: number;
}

/** Parameters accepted by `workflow_run_start`. */
export interface StartRunParams {
  definitionId: string;
  /** Run input, e.g. `{ question: "..." }` or `{ modelOverride: "..." }`. */
  input?: Record<string, unknown>;
  /** Originating thread id, for lineage. */
  parentThreadId?: string;
}

// ---------------------------------------------------------------------------
// Cost / concurrency safety gate (#3375 AC: high-cost runs require approval).
// ---------------------------------------------------------------------------

/**
 * Why a run is considered high-cost / high-concurrency and therefore gated
 * behind explicit approval. Stable codes — the UI maps each to a localized
 * sentence so the user understands what they're approving.
 */
export type WorkflowCostReason = 'non_read_only_tier' | 'high_concurrency' | 'high_children';

export interface WorkflowCostAssessment {
  /** True when the definition must be explicitly approved before starting. */
  requiresApproval: boolean;
  /** Machine-readable reasons (empty when no approval required). */
  reasons: WorkflowCostReason[];
}

/**
 * Per-phase concurrency at or above this is treated as high-concurrency.
 * The builtin parallel-research workflow runs at 2, so routine read-only
 * fan-outs stay frictionless; only unusually wide phases trip the gate.
 */
export const HIGH_CONCURRENCY_THRESHOLD = 4;

/**
 * Total child-agent budget at or above this is treated as high-cost — every
 * child is a full agent turn (tokens + wall-clock), so a large cap is the
 * clearest proxy for spend in the absence of an explicit cost field.
 */
export const HIGH_CHILDREN_THRESHOLD = 8;

/**
 * Decide whether starting `def` needs explicit user approval.
 *
 * The Rust safety tiers exist on the definition but the engine does not yet
 * gate on them, so the approval decision lives here on the client: any
 * non-`read_only` tier (child agents may take actions / edit files), or a
 * wide/expensive fan-out, requires the user to confirm before we call
 * `startRun`. Read-only, low-fan-out definitions start immediately.
 */
export function assessWorkflowCost(def: WorkflowDefinition): WorkflowCostAssessment {
  const reasons: WorkflowCostReason[] = [];
  if (def.safetyTier !== 'read_only') reasons.push('non_read_only_tier');
  if (def.defaultConcurrency >= HIGH_CONCURRENCY_THRESHOLD) reasons.push('high_concurrency');
  if (def.maxChildren >= HIGH_CHILDREN_THRESHOLD) reasons.push('high_children');
  return { requiresApproval: reasons.length > 0, reasons };
}

// ---------------------------------------------------------------------------
// RPC client.
// ---------------------------------------------------------------------------

export const workflowRunsApi = {
  /** List available declarative workflow definitions (builtins, currently). */
  listDefinitions: async (): Promise<WorkflowDefinition[]> => {
    log('listDefinitions: request');
    const result = await callCoreRpc<ListDefinitionsResult>({
      method: 'openhuman.workflow_run_list_definitions',
    });
    const definitions = result?.definitions ?? [];
    log('listDefinitions: count=%d', definitions.length);
    return definitions;
  },

  /** List durable workflow runs, newest first, with optional filters. */
  listRuns: async (params: ListRunsParams = {}): Promise<WorkflowRun[]> => {
    log('listRuns: request %o', params);
    const result = await callCoreRpc<ListRunsResult>({
      method: 'openhuman.workflow_run_list',
      params,
    });
    const runs = result?.runs ?? [];
    log('listRuns: count=%d', runs.length);
    return runs;
  },

  /** Fetch a single run by id. Returns null when no such run exists. */
  getRun: async (id: string): Promise<WorkflowRun | null> => {
    log('getRun: request id=%s', id);
    const result = await callCoreRpc<MaybeRunResult>({
      method: 'openhuman.workflow_run_get',
      params: { id },
    });
    const run = result?.workflowRun ?? null;
    log('getRun: status=%s', run?.status ?? 'null');
    return run;
  },

  /** Start a run from a definition id; returns the created Running run. */
  startRun: async (params: StartRunParams): Promise<WorkflowRun> => {
    log('startRun: request definitionId=%s', params.definitionId);
    const result = await callCoreRpc<RunResult>({ method: 'openhuman.workflow_run_start', params });
    log('startRun: id=%s status=%s', result.workflowRun.id, result.workflowRun.status);
    return result.workflowRun;
  },

  /** Stop a running workflow after its current phase. Null if id unknown. */
  stopRun: async (id: string): Promise<WorkflowRun | null> => {
    log('stopRun: request id=%s', id);
    const result = await callCoreRpc<MaybeRunResult>({
      method: 'openhuman.workflow_run_stop',
      params: { id },
    });
    log('stopRun: status=%s', result?.workflowRun?.status ?? 'null');
    return result?.workflowRun ?? null;
  },

  /** Resume an interrupted workflow from the first incomplete phase. */
  resumeRun: async (id: string): Promise<WorkflowRun> => {
    log('resumeRun: request id=%s', id);
    const result = await callCoreRpc<RunResult>({
      method: 'openhuman.workflow_run_resume',
      params: { id },
    });
    log('resumeRun: status=%s', result.workflowRun.status);
    return result.workflowRun;
  },
};
