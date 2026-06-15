import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  type WorkflowDefinition,
  type WorkflowRun,
  workflowRunsApi,
} from '../../services/api/workflowRunsApi';
import IntelligenceOrchestrationTab from './IntelligenceOrchestrationTab';

vi.mock('../../services/api/workflowRunsApi', async importOriginal => {
  // Keep the real assessWorkflowCost / thresholds; mock only the RPC client.
  const actual = await importOriginal<typeof import('../../services/api/workflowRunsApi')>();
  return {
    ...actual,
    workflowRunsApi: {
      listDefinitions: vi.fn(),
      listRuns: vi.fn(),
      getRun: vi.fn(),
      startRun: vi.fn(),
      stopRun: vi.fn(),
      resumeRun: vi.fn(),
    },
  };
});

// i18n → echo the key so assertions can target stable strings.
vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

const api = vi.mocked(workflowRunsApi);

function builtin(overrides: Partial<WorkflowDefinition> = {}): WorkflowDefinition {
  return {
    id: 'parallel_research_cross_check',
    name: 'Parallel research',
    description: 'desc',
    phases: [{ name: 'decompose', description: '', agentIds: ['planner'], dependsOn: [] }],
    defaultConcurrency: 2,
    maxChildren: 8, // >= threshold → approval required
    safetyTier: 'read_only',
    ...overrides,
  };
}

function startedRun(): WorkflowRun {
  return {
    id: 'wfrun-1',
    definitionId: 'parallel_research_cross_check',
    parentThreadId: null,
    input: {},
    phaseStates: { decompose: { status: 'running', outputs: [] } },
    childRunIds: [],
    status: 'running',
    summary: null,
    startedAt: '2026-01-01T00:00:00Z',
    updatedAt: '2026-01-01T00:00:00Z',
    completedAt: null,
  };
}

describe('IntelligenceOrchestrationTab — approval gating', () => {
  beforeEach(() => {
    api.listDefinitions.mockReset();
    api.listRuns.mockReset();
    api.startRun.mockReset();
    api.getRun.mockReset();
    api.listRuns.mockResolvedValue([]);
    api.getRun.mockResolvedValue(startedRun());
  });

  it('shows the approval card (not a direct start) for a high-cost definition', async () => {
    api.listDefinitions.mockResolvedValue([builtin()]);
    render(<IntelligenceOrchestrationTab />);

    fireEvent.click(await screen.findByTestId('orchestration-start-parallel_research_cross_check'));

    expect(screen.getByTestId('workflow-approval-card')).toBeInTheDocument();
    // No direct "start run" button when approval is required.
    expect(screen.queryByTestId('orchestration-confirm-start')).not.toBeInTheDocument();
    // startRun must NOT have fired yet — approval is still pending.
    expect(api.startRun).not.toHaveBeenCalled();
  });

  it('starts the run only after the approval is granted', async () => {
    api.listDefinitions.mockResolvedValue([builtin()]);
    api.startRun.mockResolvedValue(startedRun());
    render(<IntelligenceOrchestrationTab />);

    fireEvent.click(await screen.findByTestId('orchestration-start-parallel_research_cross_check'));
    fireEvent.click(screen.getByTestId('workflow-approval-approve'));

    await waitFor(() =>
      expect(api.startRun).toHaveBeenCalledWith({
        definitionId: 'parallel_research_cross_check',
        input: undefined,
      })
    );
  });

  it('starts directly (no approval card) for a cheap read-only definition', async () => {
    api.listDefinitions.mockResolvedValue([
      builtin({ id: 'cheap', name: 'Cheap', maxChildren: 3, defaultConcurrency: 2 }),
    ]);
    api.startRun.mockResolvedValue(startedRun());
    render(<IntelligenceOrchestrationTab />);

    fireEvent.click(await screen.findByTestId('orchestration-start-cheap'));
    expect(screen.queryByTestId('workflow-approval-card')).not.toBeInTheDocument();
    expect(screen.getByTestId('orchestration-confirm-start')).toBeInTheDocument();
  });
});
