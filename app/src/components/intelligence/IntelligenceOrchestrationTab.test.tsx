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

const cheap = () => builtin({ id: 'cheap', name: 'Cheap', maxChildren: 3, defaultConcurrency: 2 });

describe('IntelligenceOrchestrationTab — load + empty states', () => {
  beforeEach(() => {
    api.listDefinitions.mockReset();
    api.listRuns.mockReset();
    api.startRun.mockReset();
    api.getRun.mockReset();
    api.stopRun.mockReset();
    api.resumeRun.mockReset();
  });

  it('shows the error state when loading fails and retries on click', async () => {
    api.listDefinitions.mockRejectedValueOnce(new Error('rpc boom'));
    api.listRuns.mockResolvedValue([]);
    render(<IntelligenceOrchestrationTab />);

    // load() catch path (lines 79-81) + error render (224) + retry button (231-233).
    expect(await screen.findByText(/orchestration.failedToLoad/)).toBeInTheDocument();
    expect(screen.getByText(/rpc boom/)).toBeInTheDocument();

    // Second attempt succeeds and clears the error.
    api.listDefinitions.mockResolvedValueOnce([cheap()]);
    fireEvent.click(screen.getByText('common.retry'));
    expect(await screen.findByTestId('orchestration-start-cheap')).toBeInTheDocument();
  });

  it('renders the empty definitions + empty runs placeholders', async () => {
    api.listDefinitions.mockResolvedValue([]);
    api.listRuns.mockResolvedValue([]);
    render(<IntelligenceOrchestrationTab />);

    // noDefinitions (251) + noRuns placeholders.
    expect(await screen.findByText('orchestration.noDefinitions')).toBeInTheDocument();
    expect(screen.getByText('orchestration.noRuns')).toBeInTheDocument();
  });
});

describe('IntelligenceOrchestrationTab — start flow + run list', () => {
  beforeEach(() => {
    api.listDefinitions.mockReset();
    api.listRuns.mockReset();
    api.startRun.mockReset();
    api.getRun.mockReset();
    api.stopRun.mockReset();
    api.resumeRun.mockReset();
    api.listRuns.mockResolvedValue([]);
    api.getRun.mockResolvedValue(startedRun());
  });

  it('starts a cheap run with the typed question and opens its detail view', async () => {
    api.listDefinitions.mockResolvedValue([cheap()]);
    api.startRun.mockResolvedValue(startedRun());
    render(<IntelligenceOrchestrationTab />);

    fireEvent.click(await screen.findByTestId('orchestration-start-cheap'));
    // Type a question (covers the controlled textarea onChange, line 305).
    fireEvent.change(screen.getByTestId('orchestration-question'), {
      target: { value: '  why is the sky blue?  ' },
    });
    // Direct confirm-start (lines 330, 332).
    fireEvent.click(screen.getByTestId('orchestration-confirm-start'));

    // doStart trims + forwards the question (line 161) and opens the run detail.
    await waitFor(() =>
      expect(api.startRun).toHaveBeenCalledWith({
        definitionId: 'cheap',
        input: { question: 'why is the sky blue?' },
      })
    );
    expect(await screen.findByTestId('orchestration-selected-run')).toBeInTheDocument();
    // The started run is upserted into the recent-runs list (lines 402-408).
    expect(screen.getByTestId('orchestration-run-wfrun-1')).toBeInTheDocument();
  });

  it('surfaces a start error inline (doStart catch path)', async () => {
    api.listDefinitions.mockResolvedValue([cheap()]);
    api.startRun.mockRejectedValueOnce(new Error('start failed'));
    render(<IntelligenceOrchestrationTab />);

    fireEvent.click(await screen.findByTestId('orchestration-start-cheap'));
    fireEvent.click(screen.getByTestId('orchestration-confirm-start'));

    // doStart error branch (lines 169-171) renders the inline error.
    expect(await screen.findByText(/start failed/)).toBeInTheDocument();
    // The panel stays open (no selected run).
    expect(screen.queryByTestId('orchestration-selected-run')).not.toBeInTheDocument();
  });

  it('cancels the start panel without starting a run', async () => {
    api.listDefinitions.mockResolvedValue([cheap()]);
    render(<IntelligenceOrchestrationTab />);

    fireEvent.click(await screen.findByTestId('orchestration-start-cheap'));
    expect(screen.getByTestId('orchestration-confirm-start')).toBeInTheDocument();
    // cancelStart (lines 148-150) closes the panel.
    fireEvent.click(screen.getByText('orchestration.approval.cancel'));
    await waitFor(() =>
      expect(screen.queryByTestId('orchestration-confirm-start')).not.toBeInTheDocument()
    );
    expect(api.startRun).not.toHaveBeenCalled();
  });

  it('opens a recent run, updates it via upsert, then closes it', async () => {
    api.listDefinitions.mockResolvedValue([cheap()]);
    // A pre-existing run in the recent list (drives the runs.map render, 402-408).
    api.listRuns.mockResolvedValue([startedRun()]);
    render(<IntelligenceOrchestrationTab />);

    fireEvent.click(await screen.findByTestId('orchestration-run-wfrun-1'));
    expect(await screen.findByTestId('orchestration-selected-run')).toBeInTheDocument();

    // Close the drill-in (lines 364-366).
    fireEvent.click(screen.getByTestId('orchestration-close-run'));
    await waitFor(() =>
      expect(screen.queryByTestId('orchestration-selected-run')).not.toBeInTheDocument()
    );
  });
});

describe('IntelligenceOrchestrationTab — stop / resume controls', () => {
  beforeEach(() => {
    api.listDefinitions.mockReset();
    api.listRuns.mockReset();
    api.getRun.mockReset();
    api.stopRun.mockReset();
    api.resumeRun.mockReset();
    api.listDefinitions.mockResolvedValue([cheap()]);
  });

  it('stops a running run and reflects the interrupted snapshot', async () => {
    api.listRuns.mockResolvedValue([startedRun()]);
    // First poll snapshot is still running (Stop renders); once stopped the
    // backend reports interrupted, so subsequent polls agree and don't race the
    // stop result back to running (matches the serialized poll loop).
    api.getRun
      .mockResolvedValueOnce(startedRun())
      .mockResolvedValue({ ...startedRun(), status: 'interrupted' });
    api.stopRun.mockResolvedValue({ ...startedRun(), status: 'interrupted' });
    render(<IntelligenceOrchestrationTab />);

    fireEvent.click(await screen.findByTestId('orchestration-run-wfrun-1'));
    fireEvent.click(await screen.findByTestId('workflow-run-stop'));

    // handleStop (lines 179-185) calls stopRun and upserts the result.
    await waitFor(() => expect(api.stopRun).toHaveBeenCalledWith('wfrun-1'));
    await waitFor(() =>
      expect(screen.getByTestId('workflow-run-status')).toHaveTextContent(
        'orchestration.runStatus.interrupted'
      )
    );
  });

  it('resumes an interrupted run', async () => {
    const interrupted = { ...startedRun(), status: 'interrupted' as const };
    api.listRuns.mockResolvedValue([interrupted]);
    api.getRun.mockResolvedValue(interrupted);
    api.resumeRun.mockResolvedValue({ ...startedRun(), status: 'running' });
    render(<IntelligenceOrchestrationTab />);

    fireEvent.click(await screen.findByTestId('orchestration-run-wfrun-1'));
    fireEvent.click(await screen.findByTestId('workflow-run-resume'));

    // handleResume (lines 197-203) calls resumeRun and upserts the result.
    await waitFor(() => expect(api.resumeRun).toHaveBeenCalledWith('wfrun-1'));
    await waitFor(() =>
      expect(screen.getByTestId('workflow-run-status')).toHaveTextContent(
        'orchestration.runStatus.running'
      )
    );
  });

  it('swallows a stop RPC error without crashing', async () => {
    api.listRuns.mockResolvedValue([startedRun()]);
    api.getRun.mockResolvedValue(startedRun());
    api.stopRun.mockRejectedValueOnce(new Error('stop boom'));
    render(<IntelligenceOrchestrationTab />);

    fireEvent.click(await screen.findByTestId('orchestration-run-wfrun-1'));
    fireEvent.click(await screen.findByTestId('workflow-run-stop'));

    // handleStop catch path (lines 188, 190): still mounted, button re-enabled.
    await waitFor(() => expect(api.stopRun).toHaveBeenCalled());
    expect(screen.getByTestId('orchestration-selected-run')).toBeInTheDocument();
  });
});

describe('IntelligenceOrchestrationTab — polling loop', () => {
  beforeEach(() => {
    api.listDefinitions.mockReset();
    api.listRuns.mockReset();
    api.getRun.mockReset();
    api.listDefinitions.mockResolvedValue([cheap()]);
    api.listRuns.mockResolvedValue([startedRun()]);
  });

  it('polls getRun for a selected non-terminal run and upserts the fresh snapshot', async () => {
    // First poll returns a completed snapshot.
    api.getRun.mockResolvedValue({
      ...startedRun(),
      status: 'completed',
      summary: 'done',
      completedAt: '2026-01-01T00:05:00Z',
    });
    render(<IntelligenceOrchestrationTab />);

    // Select the run; the parent then starts the real 2s poll interval. We wait
    // (real timers) for the first tick rather than fight fake timers vs. the
    // interval registered inside the post-select effect.
    fireEvent.click(await screen.findByTestId('orchestration-run-wfrun-1'));
    expect(await screen.findByTestId('orchestration-selected-run')).toBeInTheDocument();

    // tick() runs after POLL_INTERVAL_MS (lines 117-121); upsert flips status.
    await waitFor(() => expect(api.getRun).toHaveBeenCalledWith('wfrun-1'), { timeout: 4000 });
    await waitFor(() =>
      expect(screen.getByTestId('workflow-run-status')).toHaveTextContent(
        'orchestration.runStatus.completed'
      )
    );
  });

  it('swallows a poll error (tick catch path)', async () => {
    api.getRun.mockRejectedValue(new Error('poll boom'));
    render(<IntelligenceOrchestrationTab />);

    fireEvent.click(await screen.findByTestId('orchestration-run-wfrun-1'));
    expect(await screen.findByTestId('orchestration-selected-run')).toBeInTheDocument();

    // tick catch (line 123): poll rejects, the run stays open with no crash.
    await waitFor(() => expect(api.getRun).toHaveBeenCalled(), { timeout: 4000 });
    expect(screen.getByTestId('orchestration-selected-run')).toBeInTheDocument();
  });
});
