/**
 * FlowApprovalRequestCard (flow-approval surface — chat) — rendering +
 * decision contract. Mirrors `ApprovalRequestCard.test.tsx`'s approach: mocks
 * `decideApproval` directly rather than the underlying RPC client.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { FlowApprovalRequest } from '../../../hooks/useFlowApprovalRequests';
import { decideApproval } from '../../../services/api/approvalApi';
import FlowApprovalRequestCard from '../FlowApprovalRequestCard';

vi.mock('../../../services/api/approvalApi', () => ({ decideApproval: vi.fn() }));

const REQUEST: FlowApprovalRequest = {
  request_id: 'req-1',
  flow_id: 'flow-1',
  run_id: 'run-1',
  tool_name: 'shell',
  summary: 'Run `shell` — rm -rf /tmp/scratch',
};

describe('FlowApprovalRequestCard', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders the summary, tool name, and flow id', () => {
    render(<FlowApprovalRequestCard request={REQUEST} onResolved={vi.fn()} />);
    expect(screen.getByText('Workflow needs approval')).toBeInTheDocument();
    expect(screen.getByText('Run `shell` — rm -rf /tmp/scratch')).toBeInTheDocument();
    expect(screen.getByText('shell')).toBeInTheDocument();
    expect(screen.getByText('flow-1')).toBeInTheDocument();
  });

  it('falls back to the generic prompt when summary is empty', () => {
    render(<FlowApprovalRequestCard request={{ ...REQUEST, summary: '' }} onResolved={vi.fn()} />);
    expect(
      screen.getByText('A workflow run wants to perform an action that needs your approval.')
    ).toBeInTheDocument();
  });

  it('Approve once routes approve_once to approval_decide and resolves', async () => {
    vi.mocked(decideApproval).mockResolvedValueOnce(undefined);
    const onResolved = vi.fn();
    render(<FlowApprovalRequestCard request={REQUEST} onResolved={onResolved} />);

    fireEvent.click(screen.getByText('Approve once'));

    expect(decideApproval).toHaveBeenCalledWith('req-1', 'approve_once');
    await waitFor(() => expect(onResolved).toHaveBeenCalledWith('req-1'));
  });

  it('Approve always routes approve_always_for_flow to approval_decide', async () => {
    vi.mocked(decideApproval).mockResolvedValueOnce(undefined);
    const onResolved = vi.fn();
    render(<FlowApprovalRequestCard request={REQUEST} onResolved={onResolved} />);

    fireEvent.click(screen.getByText('Approve always'));

    expect(decideApproval).toHaveBeenCalledWith('req-1', 'approve_always_for_flow');
    await waitFor(() => expect(onResolved).toHaveBeenCalledWith('req-1'));
  });

  it('Deny routes deny to approval_decide', async () => {
    vi.mocked(decideApproval).mockResolvedValueOnce(undefined);
    const onResolved = vi.fn();
    render(<FlowApprovalRequestCard request={REQUEST} onResolved={onResolved} />);

    fireEvent.click(screen.getByText('Deny'));

    expect(decideApproval).toHaveBeenCalledWith('req-1', 'deny');
    await waitFor(() => expect(onResolved).toHaveBeenCalledWith('req-1'));
  });

  it('keeps the prompt and shows an error when the decide RPC fails', async () => {
    vi.mocked(decideApproval).mockRejectedValueOnce(new Error('gate not installed'));
    const onResolved = vi.fn();
    render(<FlowApprovalRequestCard request={REQUEST} onResolved={onResolved} />);

    fireEvent.click(screen.getByText('Approve once'));

    await waitFor(() => {
      expect(screen.getByText(/Could not record your decision/)).toBeInTheDocument();
    });
    expect(onResolved).not.toHaveBeenCalled();
    expect(screen.getByText('Approve once')).toBeInTheDocument();
  });
});
