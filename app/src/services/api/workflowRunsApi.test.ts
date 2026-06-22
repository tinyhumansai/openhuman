import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../coreRpcClient';
import {
  assessWorkflowCost,
  type WorkflowDefinition,
  type WorkflowRun,
  workflowRunsApi,
} from './workflowRunsApi';

vi.mock('../coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const mockRpc = vi.mocked(callCoreRpc);

function def(overrides: Partial<WorkflowDefinition> = {}): WorkflowDefinition {
  return {
    id: 'parallel_research_cross_check',
    name: 'Parallel research with cross-checking',
    description: 'Decompose, research in parallel, cross-check, synthesize.',
    phases: [
      { name: 'decompose', description: '', agentIds: ['planner'], dependsOn: [] },
      {
        name: 'research',
        description: '',
        agentIds: ['researcher', 'researcher'],
        dependsOn: ['decompose'],
      },
    ],
    defaultConcurrency: 2,
    maxChildren: 8,
    safetyTier: 'read_only',
    ...overrides,
  };
}

function run(overrides: Partial<WorkflowRun> = {}): WorkflowRun {
  return {
    id: 'wfrun-1',
    definitionId: 'parallel_research_cross_check',
    parentThreadId: null,
    input: { question: 'why is the sky blue?' },
    phaseStates: {},
    childRunIds: [],
    status: 'running',
    summary: null,
    startedAt: '2026-01-01T00:00:00Z',
    updatedAt: '2026-01-01T00:00:00Z',
    completedAt: null,
    ...overrides,
  };
}

describe('assessWorkflowCost', () => {
  it('requires approval for the builtin (maxChildren >= 8)', () => {
    const result = assessWorkflowCost(def({ maxChildren: 8, defaultConcurrency: 2 }));
    expect(result.requiresApproval).toBe(true);
    expect(result.reasons).toContain('high_children');
  });

  it('does not require approval for a small read-only fan-out', () => {
    const result = assessWorkflowCost(
      def({ maxChildren: 4, defaultConcurrency: 2, safetyTier: 'read_only' })
    );
    expect(result.requiresApproval).toBe(false);
    expect(result.reasons).toHaveLength(0);
  });

  it('flags non-read-only tiers', () => {
    const result = assessWorkflowCost(
      def({ maxChildren: 2, defaultConcurrency: 1, safetyTier: 'edit_capable' })
    );
    expect(result.requiresApproval).toBe(true);
    expect(result.reasons).toContain('non_read_only_tier');
  });

  it('flags high concurrency', () => {
    const result = assessWorkflowCost(
      def({ maxChildren: 2, defaultConcurrency: 4, safetyTier: 'read_only' })
    );
    expect(result.requiresApproval).toBe(true);
    expect(result.reasons).toContain('high_concurrency');
  });
});

describe('workflowRunsApi', () => {
  beforeEach(() => mockRpc.mockReset());

  it('listDefinitions calls the RPC and returns the array', async () => {
    mockRpc.mockResolvedValueOnce({ definitions: [def()], count: 1 });
    const defs = await workflowRunsApi.listDefinitions();
    expect(mockRpc).toHaveBeenCalledWith({ method: 'openhuman.workflow_run_list_definitions' });
    expect(defs).toHaveLength(1);
    expect(defs[0].id).toBe('parallel_research_cross_check');
  });

  it('listDefinitions tolerates a missing array', async () => {
    mockRpc.mockResolvedValueOnce({});
    expect(await workflowRunsApi.listDefinitions()).toEqual([]);
  });

  it('listRuns forwards filters and returns the runs array', async () => {
    mockRpc.mockResolvedValueOnce({ runs: [run()], count: 1 });
    const runs = await workflowRunsApi.listRuns({ limit: 50, status: 'running' });
    expect(mockRpc).toHaveBeenCalledWith({
      method: 'openhuman.workflow_run_list',
      params: { limit: 50, status: 'running' },
    });
    expect(runs).toHaveLength(1);
    expect(runs[0].id).toBe('wfrun-1');
  });

  it('listRuns defaults params and tolerates a missing array', async () => {
    mockRpc.mockResolvedValueOnce({});
    const runs = await workflowRunsApi.listRuns();
    expect(mockRpc).toHaveBeenCalledWith({ method: 'openhuman.workflow_run_list', params: {} });
    expect(runs).toEqual([]);
  });

  it('getRun returns the run when present', async () => {
    mockRpc.mockResolvedValueOnce({ workflowRun: run() });
    const fetched = await workflowRunsApi.getRun('wfrun-1');
    expect(mockRpc).toHaveBeenCalledWith({
      method: 'openhuman.workflow_run_get',
      params: { id: 'wfrun-1' },
    });
    expect(fetched?.id).toBe('wfrun-1');
  });

  it('startRun forwards definitionId + input and returns the run', async () => {
    mockRpc.mockResolvedValueOnce({ workflowRun: run() });
    const started = await workflowRunsApi.startRun({
      definitionId: 'parallel_research_cross_check',
      input: { question: 'q' },
    });
    expect(mockRpc).toHaveBeenCalledWith({
      method: 'openhuman.workflow_run_start',
      params: { definitionId: 'parallel_research_cross_check', input: { question: 'q' } },
    });
    expect(started.id).toBe('wfrun-1');
  });

  it('getRun returns null when the run is absent', async () => {
    mockRpc.mockResolvedValueOnce({ workflowRun: null });
    expect(await workflowRunsApi.getRun('nope')).toBeNull();
  });

  it('stopRun returns the updated run', async () => {
    mockRpc.mockResolvedValueOnce({ workflowRun: run({ status: 'interrupted' }) });
    const stopped = await workflowRunsApi.stopRun('wfrun-1');
    expect(mockRpc).toHaveBeenCalledWith({
      method: 'openhuman.workflow_run_stop',
      params: { id: 'wfrun-1' },
    });
    expect(stopped?.status).toBe('interrupted');
  });

  it('resumeRun returns the resumed run', async () => {
    mockRpc.mockResolvedValueOnce({ workflowRun: run({ status: 'running' }) });
    const resumed = await workflowRunsApi.resumeRun('wfrun-1');
    expect(mockRpc).toHaveBeenCalledWith({
      method: 'openhuman.workflow_run_resume',
      params: { id: 'wfrun-1' },
    });
    expect(resumed.status).toBe('running');
  });
});
