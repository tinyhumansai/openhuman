/**
 * FlowRunsSidebar (Workflows UI redesign, Piece 3 — "runs rail") — asserts the
 * compact rail renders one dot per run, a click calls the controlled
 * `onSelectRun` (selection now lives in the host, not this component), the
 * selected run's dot is visually marked, the flyout expander reveals the full
 * list and a flyout row also calls `onSelectRun` (then auto-collapses), and
 * the empty/loading states.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { FlowRun } from '../../services/api/flowsApi';
import FlowRunsSidebar from './FlowRunsSidebar';

const listFlowRuns = vi.hoisted(() => vi.fn());
vi.mock('../../services/api/flowsApi', () => ({ listFlowRuns }));

const fetchPendingApprovals = vi.hoisted(() => vi.fn());
vi.mock('../../services/api/approvalApi', () => ({ fetchPendingApprovals }));

function makeRun(overrides: Partial<FlowRun> = {}): FlowRun {
  return {
    id: 'run-1',
    flow_id: 'flow-1',
    thread_id: 'run-1',
    status: 'failed',
    started_at: '2026-07-13T18:23:00Z',
    finished_at: '2026-07-13T18:23:05Z',
    steps: [],
    pending_approvals: [],
    error: 'GMAIL_SEND_EMAIL: empty body',
    ...overrides,
  };
}

function renderSidebar(props: Partial<React.ComponentProps<typeof FlowRunsSidebar>> = {}) {
  const onSelectRun = props.onSelectRun ?? vi.fn();
  return {
    onSelectRun,
    ...render(
      <FlowRunsSidebar flowId="flow-1" selectedRunId={null} onSelectRun={onSelectRun} {...props} />
    ),
  };
}

describe('FlowRunsSidebar', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    fetchPendingApprovals.mockResolvedValue([]);
  });

  it('renders one dot per run in the compact rail', async () => {
    listFlowRuns.mockResolvedValue([makeRun(), makeRun({ id: 'run-2', thread_id: 'run-2' })]);
    renderSidebar();

    expect(await screen.findByTestId('flow-runs-sidebar-run-run-1')).toBeInTheDocument();
    expect(screen.getByTestId('flow-runs-sidebar-run-run-2')).toBeInTheDocument();
  });

  it('calls onSelectRun (controlled — no internal selection/drawer) when a dot is clicked', async () => {
    listFlowRuns.mockResolvedValue([makeRun()]);
    const { onSelectRun } = renderSidebar();

    fireEvent.click(await screen.findByTestId('flow-runs-sidebar-run-run-1'));

    expect(onSelectRun).toHaveBeenCalledWith('run-1');
    // This component no longer renders any inspector/drawer itself — that's
    // the host's job now (Piece 1: lifted `selectedRunId`).
    expect(screen.queryByTestId('flow-run-inspector-drawer')).not.toBeInTheDocument();
  });

  it('marks the selected run pressed via aria-pressed', async () => {
    listFlowRuns.mockResolvedValue([makeRun(), makeRun({ id: 'run-2', thread_id: 'run-2' })]);
    renderSidebar({ selectedRunId: 'run-2' });

    await screen.findByTestId('flow-runs-sidebar-run-run-1');
    expect(screen.getByTestId('flow-runs-sidebar-run-run-1')).toHaveAttribute(
      'aria-pressed',
      'false'
    );
    expect(screen.getByTestId('flow-runs-sidebar-run-run-2')).toHaveAttribute(
      'aria-pressed',
      'true'
    );
  });

  it('shows the empty state when there are no runs', async () => {
    listFlowRuns.mockResolvedValue([]);
    renderSidebar();

    expect(await screen.findByTestId('flow-runs-sidebar-empty')).toBeInTheDocument();
  });

  it('opens a flyout with the full run list on demand, and collapses it again', async () => {
    listFlowRuns.mockResolvedValue([makeRun()]);
    renderSidebar();
    await screen.findByTestId('flow-runs-sidebar-run-run-1');

    expect(screen.queryByTestId('flow-runs-sidebar-flyout')).not.toBeInTheDocument();

    fireEvent.click(screen.getByTestId('flow-runs-sidebar-expand'));
    expect(screen.getByTestId('flow-runs-sidebar-flyout')).toBeInTheDocument();
    expect(screen.getByTestId('flow-runs-flyout-run-run-1')).toHaveTextContent('Failed');

    fireEvent.click(screen.getByTestId('flow-runs-sidebar-expand'));
    expect(screen.queryByTestId('flow-runs-sidebar-flyout')).not.toBeInTheDocument();
  });

  it('selecting a run from the flyout calls onSelectRun and auto-collapses the flyout', async () => {
    listFlowRuns.mockResolvedValue([makeRun()]);
    const { onSelectRun } = renderSidebar();
    await screen.findByTestId('flow-runs-sidebar-run-run-1');

    fireEvent.click(screen.getByTestId('flow-runs-sidebar-expand'));
    fireEvent.click(screen.getByTestId('flow-runs-flyout-run-run-1'));

    expect(onSelectRun).toHaveBeenCalledWith('run-1');
    expect(screen.queryByTestId('flow-runs-sidebar-flyout')).not.toBeInTheDocument();
  });

  it('refetches when the refresh button is clicked', async () => {
    listFlowRuns.mockResolvedValue([makeRun()]);
    renderSidebar();
    await screen.findByTestId('flow-runs-sidebar-run-run-1');
    expect(listFlowRuns).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByTestId('flow-runs-sidebar-refresh'));
    await waitFor(() => expect(listFlowRuns).toHaveBeenCalledTimes(2));
  });

  it('shows an "Awaiting approval" tooltip title for a running run halted at an approval gate', async () => {
    listFlowRuns.mockResolvedValue([makeRun({ status: 'running' })]);
    fetchPendingApprovals.mockResolvedValue([
      {
        request_id: 'req-1',
        tool_name: 'SLACK_SEND_MESSAGE',
        action_summary: 'Send Slack message',
        args_redacted: {},
        session_id: 'session-1',
        created_at: '2026-07-13T18:23:00Z',
        expires_at: null,
        source_context: { kind: 'flow', flow_id: 'flow-1', run_id: 'run-1' },
      },
    ]);
    renderSidebar();

    const dot = await screen.findByTestId('flow-runs-sidebar-run-run-1');
    // Tooltip label rides the native `title` fallback (see `ui/Tooltip.tsx`).
    await waitFor(() =>
      expect(dot).toHaveAttribute('title', expect.stringContaining('Awaiting approval'))
    );
  });
});
