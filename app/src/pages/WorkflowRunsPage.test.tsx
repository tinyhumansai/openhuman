import { act, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import WorkflowRunsPage from './WorkflowRunsPage';

vi.mock('../lib/i18n/I18nContext', () => ({
  useT: () => ({ t: (key: string, fallback?: string) => fallback ?? key }),
}));

const navigateMock = vi.hoisted(() => vi.fn());
vi.mock('react-router-dom', () => ({ useNavigate: () => navigateMock }));

const listAllFlowRuns = vi.hoisted(() => vi.fn());
const listFlows = vi.hoisted(() => vi.fn());
vi.mock('../services/api/flowsApi', () => ({
  listAllFlowRuns: (...a: unknown[]) => listAllFlowRuns(...a),
  listFlows: (...a: unknown[]) => listFlows(...a),
}));

const fetchPendingApprovals = vi.hoisted(() => vi.fn());
vi.mock('../services/api/approvalApi', () => ({ fetchPendingApprovals }));

// PanelPage + LoadingState pull i18n/redux we don't need — stub to bare markup.
vi.mock('../components/layout/PanelPage', () => ({
  default: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
}));
vi.mock('../components/ui/LoadingState', () => ({
  CenteredLoadingState: ({ label }: { label: string }) => <div>{label}</div>,
  ErrorBanner: ({ message }: { message: string }) => <div>{message}</div>,
}));

describe('WorkflowRunsPage', () => {
  beforeEach(() => {
    navigateMock.mockReset();
    listAllFlowRuns.mockReset();
    listFlows.mockReset();
    fetchPendingApprovals.mockReset();
    fetchPendingApprovals.mockResolvedValue([]);
  });

  it('renders aggregate runs with their workflow name and status', async () => {
    listAllFlowRuns.mockResolvedValue([
      { id: 'r1', flow_id: 'f1', status: 'completed', started_at: '2026-01-01T00:00:00Z' },
      {
        id: 'r2',
        flow_id: 'f2',
        status: 'failed',
        started_at: '2026-01-02T00:00:00Z',
        error: 'boom',
      },
    ]);
    listFlows.mockResolvedValue([
      { id: 'f1', name: 'Daily digest' },
      { id: 'f2', name: 'Auto reply' },
    ]);

    render(<WorkflowRunsPage />);

    await waitFor(() => expect(screen.getByTestId('workflow-runs-list')).toBeInTheDocument());
    expect(screen.getByText('Daily digest')).toBeInTheDocument();
    expect(screen.getByText('Auto reply')).toBeInTheDocument();
    // A failed run surfaces its error inline.
    expect(screen.getByText('boom')).toBeInTheDocument();
  });

  it('shows the empty state when there are no runs', async () => {
    listAllFlowRuns.mockResolvedValue([]);
    listFlows.mockResolvedValue([]);

    render(<WorkflowRunsPage />);

    await waitFor(() => expect(screen.getByTestId('workflow-runs-empty')).toBeInTheDocument());
  });

  it('shows an error banner when the fetch fails', async () => {
    listAllFlowRuns.mockRejectedValue(new Error('rpc down'));
    listFlows.mockResolvedValue([]);

    render(<WorkflowRunsPage />);

    await waitFor(() => expect(screen.getByText('rpc down')).toBeInTheDocument());
  });

  it('shows "pending approval" for a running run halted at a matching flow approval gate', async () => {
    listAllFlowRuns.mockResolvedValue([
      { id: 'r1', flow_id: 'f1', status: 'running', started_at: '2026-01-01T00:00:00Z' },
    ]);
    listFlows.mockResolvedValue([{ id: 'f1', name: 'Daily digest' }]);
    fetchPendingApprovals.mockResolvedValue([
      {
        request_id: 'req-1',
        tool_name: 'SLACK_SEND_MESSAGE',
        action_summary: 'Send Slack message',
        args_redacted: {},
        session_id: 'session-1',
        created_at: '2026-01-01T00:00:00Z',
        expires_at: null,
        source_context: { kind: 'flow', flow_id: 'f1', run_id: 'r1' },
      },
    ]);

    render(<WorkflowRunsPage />);

    const row = await screen.findByTestId('workflow-run-r1');
    await waitFor(() => expect(row).toHaveTextContent('pending approval'));
  });

  it('leaves a running run without a matching flow approval labeled "running"', async () => {
    listAllFlowRuns.mockResolvedValue([
      { id: 'r1', flow_id: 'f1', status: 'running', started_at: '2026-01-01T00:00:00Z' },
    ]);
    listFlows.mockResolvedValue([{ id: 'f1', name: 'Daily digest' }]);
    fetchPendingApprovals.mockResolvedValue([]);

    render(<WorkflowRunsPage />);

    const row = await screen.findByTestId('workflow-run-r1');
    await waitFor(() => expect(row).toHaveTextContent('running'));
  });

  describe('live refresh (useFlowRunsLiveRefresh integration)', () => {
    afterEach(() => {
      vi.useRealTimers();
    });

    it('re-fetches just the runs (not listFlows) on the live-refresh poll while a run is active', async () => {
      vi.useFakeTimers();
      listAllFlowRuns
        .mockResolvedValueOnce([
          { id: 'r1', flow_id: 'f1', status: 'running', started_at: '2026-01-01T00:00:00Z' },
        ])
        .mockResolvedValueOnce([
          { id: 'r1', flow_id: 'f1', status: 'completed', started_at: '2026-01-01T00:00:00Z' },
        ]);
      listFlows.mockResolvedValue([{ id: 'f1', name: 'Daily digest' }]);

      render(<WorkflowRunsPage />);
      await act(async () => {
        await Promise.resolve();
      });
      expect(screen.getByTestId('workflow-runs-list')).toBeInTheDocument();
      expect(listAllFlowRuns).toHaveBeenCalledTimes(1);
      expect(listFlows).toHaveBeenCalledTimes(1);

      // The 5s poll fallback fires `refetchRuns` while the one run is still 'running'.
      await act(async () => {
        vi.advanceTimersByTime(5_000);
        await Promise.resolve();
      });

      // The poll fallback re-fetched just the runs — `listFlows` is not called again.
      expect(listAllFlowRuns).toHaveBeenCalledTimes(2);
      expect(listFlows).toHaveBeenCalledTimes(1);
    });

    it('does not surface an error banner when a background refetch fails', async () => {
      vi.useFakeTimers();
      listAllFlowRuns
        .mockResolvedValueOnce([
          { id: 'r1', flow_id: 'f1', status: 'running', started_at: '2026-01-01T00:00:00Z' },
        ])
        .mockRejectedValueOnce(new Error('transient rpc blip'));
      listFlows.mockResolvedValue([{ id: 'f1', name: 'Daily digest' }]);

      render(<WorkflowRunsPage />);
      await act(async () => {
        await Promise.resolve();
      });
      expect(screen.getByTestId('workflow-runs-list')).toBeInTheDocument();

      await act(async () => {
        vi.advanceTimersByTime(5_000);
        await Promise.resolve();
      });

      expect(listAllFlowRuns).toHaveBeenCalledTimes(2);
      // The transient background failure is logged only — the list stays as-is,
      // no error banner from the (unrelated) `load` error state.
      expect(screen.queryByText('transient rpc blip')).not.toBeInTheDocument();
      expect(screen.getByTestId('workflow-runs-list')).toBeInTheDocument();
    });
  });
});
