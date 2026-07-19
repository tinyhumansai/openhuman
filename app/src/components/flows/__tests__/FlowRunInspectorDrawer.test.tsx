/**
 * FlowRunInspectorDrawer (issue B3b) — rendering contract.
 *
 * Asserts: renders null when `runId` is null; loading state; renders fetched
 * run data (status pill, steps, expandable output, port pill); error state;
 * actionable pending-approval cards when `status === 'pending_approval'`
 * (flow-approval surface — run details); run.error banner; Escape and
 * backdrop both close; close button calls `onClose`.
 *
 * Mocks `useFlowRunPoller` and `useFlowPendingApprovals` directly rather than
 * the underlying RPC client — their own poll contracts are covered by
 * `hooks/__tests__/useFlowRunPoller.test.ts` and
 * `hooks/__tests__/useFlowPendingApprovals.test.ts`.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { PendingApproval } from '../../../services/api/approvalApi';
import type { FlowRun } from '../../../services/api/flowsApi';
import { store } from '../../../store';
import { FlowRunInspectorDrawer } from '../FlowRunInspectorDrawer';

const useFlowRunPoller = vi.hoisted(() => vi.fn());
vi.mock('../../../hooks/useFlowRunPoller', () => ({ useFlowRunPoller }));

const useFlowPendingApprovals = vi.hoisted(() => vi.fn());
vi.mock('../../../hooks/useFlowPendingApprovals', () => ({ useFlowPendingApprovals }));

function makeApproval(overrides: Partial<PendingApproval> = {}): PendingApproval {
  return {
    request_id: 'req-1',
    tool_name: 'shell',
    action_summary: 'Run `shell` — rm -rf /tmp/scratch',
    args_redacted: {},
    session_id: 'session-1',
    created_at: '2026-01-01T00:00:00Z',
    expires_at: null,
    source_context: { kind: 'flow', flow_id: 'flow-1', run_id: 'thread-1' },
    ...overrides,
  };
}

function makeRun(overrides: Partial<FlowRun> = {}): FlowRun {
  return {
    id: 'thread-1',
    flow_id: 'flow-1',
    thread_id: 'thread-1',
    status: 'running',
    started_at: '2026-01-01T00:00:00Z',
    steps: [
      { node_id: 'fetch-data', output: { rows: 3 } },
      { node_id: 'branch', output: 'ok', port: 'true' },
    ],
    pending_approvals: [],
    ...overrides,
  };
}

function renderDrawer(runId: string | null, onClose: () => void) {
  return render(
    <Provider store={store}>
      <FlowRunInspectorDrawer runId={runId} onClose={onClose} />
    </Provider>
  );
}

describe('FlowRunInspectorDrawer', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // Default: no pending approvals, no decide-in-flight. Tests that exercise
    // the actionable approval cards override this explicitly.
    useFlowPendingApprovals.mockReturnValue({
      approvals: [],
      decidingId: null,
      error: null,
      decide: vi.fn(),
    });
  });

  it('renders null when runId is null', () => {
    useFlowRunPoller.mockReturnValue({ run: null, loading: false, error: null });
    const { container } = renderDrawer(null, vi.fn());
    expect(container).toBeEmptyDOMElement();
    expect(useFlowRunPoller).toHaveBeenCalledWith(null);
  });

  it('shows a loading state before data resolves', () => {
    useFlowRunPoller.mockReturnValue({ run: null, loading: true, error: null });
    renderDrawer('thread-1', vi.fn());
    expect(screen.getByTestId('flow-run-inspector-loading')).toBeInTheDocument();
  });

  it('renders the run status pill and step list once data resolves', () => {
    useFlowRunPoller.mockReturnValue({ run: makeRun(), loading: false, error: null });
    renderDrawer('thread-1', vi.fn());

    expect(screen.getByTestId('flow-run-status-pill')).toHaveTextContent('Running');
    expect(screen.getByTestId('flow-run-steps')).toBeInTheDocument();
    expect(screen.getByText('fetch-data')).toBeInTheDocument();
    expect(screen.getByText('branch')).toBeInTheDocument();
    expect(screen.getByTestId('flow-run-step-port-1')).toHaveTextContent('true');
  });

  it('expands a step to reveal its output in the per-item data browser', () => {
    useFlowRunPoller.mockReturnValue({ run: makeRun(), loading: false, error: null });
    renderDrawer('thread-1', vi.fn());

    // Data browser lives inside a collapsed <details> until expanded.
    expect(screen.queryByTestId('flow-run-step-0-data-browser')).not.toBeVisible();
    fireEvent.click(screen.getAllByText('Show raw output')[0]);

    // Default table view shows the `rows` column and its value.
    const browser = screen.getByTestId('flow-run-step-0-data-browser');
    expect(browser).toBeVisible();
    expect(screen.getByTestId('flow-run-step-0-table')).toBeInTheDocument();
    expect(screen.getByTestId('flow-run-step-0-row-0')).toHaveTextContent('3');

    // Toggling to JSON shows the pretty-printed payload.
    fireEvent.click(screen.getByTestId('flow-run-step-0-view-json'));
    expect(screen.getByTestId('flow-run-step-0-json').textContent).toContain('"rows": 3');
  });

  // Issue B20 — a plain-language summary is the always-visible primary view;
  // raw Composio/tool JSON (costUsd, labelIds, markdownFormatted, …) and the
  // full flow:/run: ids stay behind "Show raw output" / hover-only.
  it('shows a plain-language success summary without raw technical fields, hidden behind "Show raw output"', () => {
    useFlowRunPoller.mockReturnValue({
      run: makeRun({
        steps: [
          {
            node_id: 'send-summary',
            status: 'success',
            output: {
              summary: 'Sent your daily email summary — no new emails today.',
              has_important: false,
              costUsd: 0.0004,
              labelIds: ['INBOX'],
              markdownFormatted: '**no new emails**',
            },
          },
        ],
      }),
      loading: false,
      error: null,
    });
    renderDrawer('thread-1', vi.fn());

    // Primary view: the human summary, visible without any interaction.
    expect(screen.getByTestId('flow-run-step-summary-0')).toHaveTextContent(
      'Sent your daily email summary — no new emails today.'
    );
    // Technical/internal fields exist only inside the collapsed raw-output
    // disclosure — not visible in the always-visible primary view.
    expect(screen.getByText('costUsd')).not.toBeVisible();
    expect(screen.getByText('labelIds')).not.toBeVisible();
    expect(screen.getByText('markdownFormatted')).not.toBeVisible();

    // Raw output (with the technical fields) only appears once expanded.
    expect(screen.queryByTestId('flow-run-step-0-data-browser')).not.toBeVisible();
    fireEvent.click(screen.getAllByText('Show raw output')[0]);
    fireEvent.click(screen.getByTestId('flow-run-step-0-view-json'));
    expect(screen.getByTestId('flow-run-step-0-json').textContent).toContain('costUsd');
  });

  it('shows a plain-language failure summary for a failed step', () => {
    useFlowRunPoller.mockReturnValue({
      run: makeRun({
        steps: [
          {
            node_id: 'send-email',
            status: 'success',
            output: { data: null, successful: false, error: 'recipient address invalid' },
          },
        ],
      }),
      loading: false,
      error: null,
    });
    renderDrawer('thread-1', vi.fn());

    expect(screen.getByTestId('flow-run-step-summary-0')).toHaveTextContent(
      "Couldn't complete: recipient address invalid"
    );
  });

  it('shows short-form run/flow ids in the header, full value only on hover title', () => {
    useFlowRunPoller.mockReturnValue({
      run: makeRun({ flow_id: 'flow-abcdefgh12345', thread_id: 'thread-abcdefgh12345' }),
      loading: false,
      error: null,
    });
    renderDrawer('thread-1', vi.fn());

    expect(screen.queryByText('flow-abcdefgh12345')).not.toBeInTheDocument();
    expect(screen.queryByText('thread-abcdefgh12345')).not.toBeInTheDocument();
    expect(screen.getByTitle('flow-abcdefgh12345')).toHaveTextContent('flow-abc');
    expect(screen.getByTitle('thread-abcdefgh12345')).toHaveTextContent('thread-a');
  });

  it('shows an error state when the poller reports an error', () => {
    useFlowRunPoller.mockReturnValue({ run: null, loading: false, error: 'network down' });
    renderDrawer('thread-1', vi.fn());
    expect(screen.getByTestId('flow-run-inspector-error')).toHaveTextContent('network down');
  });

  // ── Actionable pending-approval gates (flow-approval surface — run
  // details). Replaces the old read-only "N node(s) awaiting approval"
  // banner with Approve once / Approve always / Deny cards wired to
  // `openhuman.approval_decide` via `useFlowPendingApprovals`.
  it('polls scoped to this run only while pending_approval/running, passing null otherwise', () => {
    useFlowRunPoller.mockReturnValue({
      run: makeRun({ status: 'pending_approval', flow_id: 'flow-9', thread_id: 'thread-9' }),
      loading: false,
      error: null,
    });
    renderDrawer('thread-9', vi.fn());
    expect(useFlowPendingApprovals).toHaveBeenCalledWith('flow-9', 'thread-9');
  });

  it('passes null/null to stop polling once the run is terminal', () => {
    useFlowRunPoller.mockReturnValue({
      run: makeRun({ status: 'completed' }),
      loading: false,
      error: null,
    });
    renderDrawer('thread-1', vi.fn());
    expect(useFlowPendingApprovals).toHaveBeenCalledWith(null, null);
  });

  it('renders an actionable card per pending approval scoped to this run', () => {
    useFlowRunPoller.mockReturnValue({
      run: makeRun({ status: 'pending_approval' }),
      loading: false,
      error: null,
    });
    useFlowPendingApprovals.mockReturnValue({
      approvals: [makeApproval({ request_id: 'req-a' }), makeApproval({ request_id: 'req-b' })],
      decidingId: null,
      error: null,
      decide: vi.fn(),
    });
    renderDrawer('thread-1', vi.fn());
    expect(screen.getByTestId('flow-run-pending-approval-req-a')).toBeInTheDocument();
    expect(screen.getByTestId('flow-run-pending-approval-req-b')).toBeInTheDocument();
  });

  it('does not show any approval card for a running run with no pending approvals', () => {
    useFlowRunPoller.mockReturnValue({ run: makeRun(), loading: false, error: null });
    renderDrawer('thread-1', vi.fn());
    expect(screen.queryByTestId('flow-run-pending-approvals')).not.toBeInTheDocument();
  });

  it("routes Approve once / Approve always / Deny to the hook's decide() with the request id", () => {
    const decide = vi.fn().mockResolvedValue(undefined);
    useFlowRunPoller.mockReturnValue({
      run: makeRun({ status: 'pending_approval' }),
      loading: false,
      error: null,
    });
    useFlowPendingApprovals.mockReturnValue({
      approvals: [makeApproval({ request_id: 'req-a' })],
      decidingId: null,
      error: null,
      decide,
    });
    renderDrawer('thread-1', vi.fn());

    fireEvent.click(screen.getByTestId('flow-run-pending-approval-approve-req-a'));
    expect(decide).toHaveBeenCalledWith('req-a', 'approve_once');

    fireEvent.click(screen.getByTestId('flow-run-pending-approval-always-req-a'));
    expect(decide).toHaveBeenCalledWith('req-a', 'approve_always_for_flow');

    fireEvent.click(screen.getByTestId('flow-run-pending-approval-deny-req-a'));
    expect(decide).toHaveBeenCalledWith('req-a', 'deny');
  });

  it('shows the polling error message when useFlowPendingApprovals reports one', () => {
    useFlowRunPoller.mockReturnValue({
      run: makeRun({ status: 'pending_approval' }),
      loading: false,
      error: null,
    });
    useFlowPendingApprovals.mockReturnValue({
      approvals: [makeApproval()],
      decidingId: null,
      error: 'network down',
      decide: vi.fn(),
    });
    renderDrawer('thread-1', vi.fn());
    expect(screen.getByTestId('flow-run-pending-approvals-error')).toBeInTheDocument();
  });

  it('shows the run.error banner when present', () => {
    useFlowRunPoller.mockReturnValue({
      run: makeRun({ status: 'failed', error: 'node crashed' }),
      loading: false,
      error: null,
    });
    renderDrawer('thread-1', vi.fn());
    expect(screen.getByTestId('flow-run-error-banner')).toHaveTextContent('node crashed');
  });

  it('calls onClose when the close button is clicked', () => {
    useFlowRunPoller.mockReturnValue({ run: makeRun(), loading: false, error: null });
    const onClose = vi.fn();
    renderDrawer('thread-1', onClose);
    fireEvent.click(screen.getByTestId('flow-run-inspector-close'));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('calls onClose when the backdrop is clicked', () => {
    useFlowRunPoller.mockReturnValue({ run: makeRun(), loading: false, error: null });
    const onClose = vi.fn();
    renderDrawer('thread-1', onClose);
    fireEvent.click(screen.getByTestId('flow-run-inspector-backdrop'));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('calls onClose when Escape is pressed', () => {
    useFlowRunPoller.mockReturnValue({ run: makeRun(), loading: false, error: null });
    const onClose = vi.fn();
    renderDrawer('thread-1', onClose);
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  // ── "Fix with agent" repair entry point (Phase 5c) ────────────────────────
  it('shows "Fix with agent" only for a failed run when the handler is provided', () => {
    useFlowRunPoller.mockReturnValue({
      run: makeRun({ status: 'failed', error: 'HTTP 500' }),
      loading: false,
      error: null,
    });
    render(
      <Provider store={store}>
        <FlowRunInspectorDrawer runId="thread-1" onClose={vi.fn()} onFixWithAgent={vi.fn()} />
      </Provider>
    );
    expect(screen.getByTestId('flow-run-fix-with-agent')).toBeInTheDocument();
  });

  it('hides "Fix with agent" for a non-failed run', () => {
    useFlowRunPoller.mockReturnValue({
      run: makeRun({ status: 'completed' }),
      loading: false,
      error: null,
    });
    render(
      <Provider store={store}>
        <FlowRunInspectorDrawer runId="thread-1" onClose={vi.fn()} onFixWithAgent={vi.fn()} />
      </Provider>
    );
    expect(screen.queryByTestId('flow-run-fix-with-agent')).not.toBeInTheDocument();
  });

  it('hands the failed run context up when "Fix with agent" is clicked', () => {
    useFlowRunPoller.mockReturnValue({
      run: makeRun({ status: 'failed', error: 'HTTP 500', flow_id: 'flow-42', thread_id: 'run-9' }),
      loading: false,
      error: null,
    });
    const onFixWithAgent = vi.fn();
    render(
      <Provider store={store}>
        <FlowRunInspectorDrawer runId="run-9" onClose={vi.fn()} onFixWithAgent={onFixWithAgent} />
      </Provider>
    );
    fireEvent.click(screen.getByTestId('flow-run-fix-with-agent'));
    expect(onFixWithAgent).toHaveBeenCalledWith(
      expect.objectContaining({ flowId: 'flow-42', runId: 'run-9', error: 'HTTP 500' })
    );
  });
});
