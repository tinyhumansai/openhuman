import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { PendingApproval } from '../../services/api/approvalApi';
import { FlowRunPendingApprovalCard } from './FlowRunPendingApprovalCard';

const APPROVAL: PendingApproval = {
  request_id: 'request-1',
  tool_name: 'shell',
  action_summary: 'Run the release command',
  args_redacted: {},
  session_id: 'session-1',
  created_at: '2026-07-24T00:00:00Z',
  expires_at: null,
  source_context: { kind: 'flow', flow_id: 'flow-1', run_id: 'run-1' },
};

describe('FlowRunPendingApprovalCard', () => {
  it('renders run approval copy and preserves its test selectors', () => {
    render(<FlowRunPendingApprovalCard approval={APPROVAL} deciding={false} onDecide={vi.fn()} />);

    expect(screen.getByRole('alertdialog', { name: 'Pending approvals' })).toHaveAttribute(
      'data-testid',
      'flow-run-pending-approval-request-1'
    );
    expect(screen.getByText('Run the release command')).toBeInTheDocument();
    expect(screen.getByText('shell')).toBeInTheDocument();
    expect(screen.getByTestId('flow-run-pending-approval-approve-request-1')).toHaveTextContent(
      'Approve once'
    );
    expect(screen.getByTestId('flow-run-pending-approval-always-request-1')).toHaveTextContent(
      'Approve always'
    );
    expect(screen.getByTestId('flow-run-pending-approval-deny-request-1')).toHaveTextContent(
      'Deny'
    );
    expect(screen.getByText('🔒')).toHaveClass('text-sm');
    expect(screen.getByText('🔒')).not.toHaveClass('text-amber-700');
    expect(screen.getByTestId('flow-run-pending-approval-approve-request-1')).toHaveClass('h-6');
    expect(
      screen.getByTestId('flow-run-pending-approval-approve-request-1').parentElement
    ).toHaveClass('mt-2', 'gap-1.5');
    expect(screen.getByTestId('flow-run-pending-approval-deny-request-1')).toHaveClass(
      'border-line-strong'
    );
    expect(screen.getByTestId('flow-run-pending-approval-deny-request-1').className).not.toMatch(
      /coral/
    );
  });

  it.each([
    ['flow-run-pending-approval-approve-request-1', 'approve_once'],
    ['flow-run-pending-approval-always-request-1', 'approve_always_for_flow'],
    ['flow-run-pending-approval-deny-request-1', 'deny'],
  ] as const)('maps %s to %s', (testId, decision) => {
    const onDecide = vi.fn().mockResolvedValue(undefined);
    render(<FlowRunPendingApprovalCard approval={APPROVAL} deciding={false} onDecide={onDecide} />);

    fireEvent.click(screen.getByTestId(testId));
    expect(onDecide).toHaveBeenCalledWith(decision);
  });

  it('shows the active busy label only and disables every action while deciding', () => {
    const onDecide = vi.fn().mockReturnValue(new Promise<void>(() => undefined));
    const { rerender } = render(
      <FlowRunPendingApprovalCard approval={APPROVAL} deciding={false} onDecide={onDecide} />
    );

    fireEvent.click(screen.getByTestId('flow-run-pending-approval-always-request-1'));
    rerender(<FlowRunPendingApprovalCard approval={APPROVAL} deciding onDecide={onDecide} />);

    expect(screen.getByTestId('flow-run-pending-approval-approve-request-1')).toBeDisabled();
    expect(screen.getByTestId('flow-run-pending-approval-always-request-1')).toBeDisabled();
    expect(screen.getByTestId('flow-run-pending-approval-deny-request-1')).toBeDisabled();
    expect(screen.getByTestId('flow-run-pending-approval-always-request-1')).toHaveTextContent(
      'Working…'
    );
    expect(screen.getByTestId('flow-run-pending-approval-approve-request-1')).toHaveTextContent(
      'Approve once'
    );
    expect(screen.getByTestId('flow-run-pending-approval-deny-request-1')).toHaveTextContent(
      'Deny'
    );
  });

  it('disables every action without showing a busy label when already deciding on first render', () => {
    render(<FlowRunPendingApprovalCard approval={APPROVAL} deciding onDecide={vi.fn()} />);

    expect(screen.getByTestId('flow-run-pending-approval-approve-request-1')).toBeDisabled();
    expect(screen.getByTestId('flow-run-pending-approval-always-request-1')).toBeDisabled();
    expect(screen.getByTestId('flow-run-pending-approval-deny-request-1')).toBeDisabled();
    expect(screen.queryByText('Working…')).not.toBeInTheDocument();
  });
});
