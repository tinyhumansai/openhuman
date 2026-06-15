import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { type WorkflowDefinition, type WorkflowRun } from '../../services/api/workflowRunsApi';
import WorkflowRunDetail from './WorkflowRunDetail';

// i18n → echo the key so assertions can target stable strings.
vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

function def(): WorkflowDefinition {
  return {
    id: 'parallel_research_cross_check',
    name: 'Parallel research',
    description: 'desc',
    phases: [
      {
        name: 'decompose',
        description: 'Break the question down.',
        agentIds: ['planner'],
        dependsOn: [],
      },
      {
        name: 'research',
        description: 'Research in parallel.',
        agentIds: ['researcher', 'researcher'],
        dependsOn: ['decompose'],
      },
      {
        name: 'synthesize',
        description: 'Synthesize.',
        agentIds: ['summarizer'],
        dependsOn: ['research'],
      },
    ],
    defaultConcurrency: 2,
    maxChildren: 8,
    safetyTier: 'read_only',
  };
}

function run(overrides: Partial<WorkflowRun> = {}): WorkflowRun {
  return {
    id: 'wfrun-1',
    definitionId: 'parallel_research_cross_check',
    parentThreadId: null,
    input: { question: 'q' },
    phaseStates: {
      decompose: {
        status: 'completed',
        outputs: [{ orchestrationId: 'orch-1', agentId: 'planner', output: 'three angles' }],
      },
      research: { status: 'running', outputs: [] },
      synthesize: { status: 'pending', outputs: [] },
    },
    childRunIds: ['orch-1'],
    status: 'running',
    summary: null,
    startedAt: '2026-01-01T00:00:00Z',
    updatedAt: '2026-01-01T00:01:00Z',
    completedAt: null,
    ...overrides,
  };
}

describe('WorkflowRunDetail — progress rendering', () => {
  it('renders the run status and every declared phase in order', () => {
    render(
      <WorkflowRunDetail definition={def()} run={run()} onStop={vi.fn()} onResume={vi.fn()} />
    );

    expect(screen.getByTestId('workflow-run-status')).toHaveTextContent(
      'orchestration.runStatus.running'
    );

    const phases = screen.getByTestId('workflow-phase-list').querySelectorAll('li');
    expect(phases).toHaveLength(3);
    expect(screen.getByTestId('workflow-phase-decompose')).toBeInTheDocument();
    expect(screen.getByTestId('workflow-phase-research')).toBeInTheDocument();
    expect(screen.getByTestId('workflow-phase-synthesize')).toBeInTheDocument();
  });

  it('reflects per-phase status from phaseStates', () => {
    render(
      <WorkflowRunDetail definition={def()} run={run()} onStop={vi.fn()} onResume={vi.fn()} />
    );
    expect(screen.getByTestId('workflow-phase-status-decompose')).toHaveTextContent(
      'orchestration.phaseStatus.completed'
    );
    expect(screen.getByTestId('workflow-phase-status-research')).toHaveTextContent(
      'orchestration.phaseStatus.running'
    );
    expect(screen.getByTestId('workflow-phase-status-synthesize')).toHaveTextContent(
      'orchestration.phaseStatus.pending'
    );
  });

  it('shows child agent refs for a phase when expanded', () => {
    render(
      <WorkflowRunDetail definition={def()} run={run()} onStop={vi.fn()} onResume={vi.fn()} />
    );
    // Outputs are collapsed by default; expand the completed phase.
    fireEvent.click(screen.getByTestId('workflow-phase-decompose').querySelector('button')!);
    const outputs = screen.getByTestId('workflow-phase-outputs-decompose');
    expect(outputs).toHaveTextContent('planner');
    expect(outputs).toHaveTextContent('orch-1');
    expect(outputs).toHaveTextContent('three angles');
  });

  it('renders the run-level child agent ref count', () => {
    render(
      <WorkflowRunDetail definition={def()} run={run()} onStop={vi.fn()} onResume={vi.fn()} />
    );
    expect(screen.getByTestId('workflow-child-refs')).toBeInTheDocument();
  });

  it('shows the final synthesis only once the run is terminal with a summary', () => {
    const { rerender } = render(
      <WorkflowRunDetail definition={def()} run={run()} onStop={vi.fn()} onResume={vi.fn()} />
    );
    expect(screen.queryByTestId('workflow-run-summary')).not.toBeInTheDocument();

    rerender(
      <WorkflowRunDetail
        definition={def()}
        run={run({
          status: 'completed',
          summary: 'A cited report.',
          completedAt: '2026-01-01T00:05:00Z',
        })}
        onStop={vi.fn()}
        onResume={vi.fn()}
      />
    );
    expect(screen.getByTestId('workflow-run-summary')).toHaveTextContent('A cited report.');
  });

  it('offers Stop while running and wires it to onStop', () => {
    const onStop = vi.fn();
    render(<WorkflowRunDetail definition={def()} run={run()} onStop={onStop} onResume={vi.fn()} />);
    expect(screen.queryByTestId('workflow-run-resume')).not.toBeInTheDocument();
    fireEvent.click(screen.getByTestId('workflow-run-stop'));
    expect(onStop).toHaveBeenCalledWith('wfrun-1');
  });

  it('offers Resume when interrupted and wires it to onResume', () => {
    const onResume = vi.fn();
    render(
      <WorkflowRunDetail
        definition={def()}
        run={run({ status: 'interrupted' })}
        onStop={vi.fn()}
        onResume={onResume}
      />
    );
    expect(screen.queryByTestId('workflow-run-stop')).not.toBeInTheDocument();
    fireEvent.click(screen.getByTestId('workflow-run-resume'));
    expect(onResume).toHaveBeenCalledWith('wfrun-1');
  });

  it('falls back to phaseStates keys + raw phase name when the definition is missing', () => {
    render(
      <WorkflowRunDetail
        definition={undefined}
        run={run({
          phaseStates: { decompose: { status: 'running', outputs: [] } },
          childRunIds: [],
        })}
        onStop={vi.fn()}
        onResume={vi.fn()}
      />
    );
    // With no definition, the phase order comes from run.phaseStates and the
    // label falls back to the raw phase name (WorkflowRunDetail.tsx:173).
    const phases = screen.getByTestId('workflow-phase-list').querySelectorAll('li');
    expect(phases).toHaveLength(1);
    expect(screen.getByTestId('workflow-phase-decompose')).toHaveTextContent('decompose');
  });

  it('renders the failure reason for a failed phase', () => {
    render(
      <WorkflowRunDetail
        definition={def()}
        run={run({
          status: 'failed',
          phaseStates: {
            decompose: { status: 'failed', outputs: [], reason: 'planner agent errored out' },
            research: { status: 'pending', outputs: [] },
            synthesize: { status: 'pending', outputs: [] },
          },
        })}
        onStop={vi.fn()}
        onResume={vi.fn()}
      />
    );
    // The failed-phase reason banner (WorkflowRunDetail.tsx:197).
    expect(screen.getByTestId('workflow-phase-decompose')).toHaveTextContent(
      'planner agent errored out'
    );
  });
});
