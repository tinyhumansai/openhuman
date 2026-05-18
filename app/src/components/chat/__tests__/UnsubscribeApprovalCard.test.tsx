import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../../../services/coreRpcClient';
import { renderWithProviders } from '../../../test/test-utils';
import { UnsubscribeApprovalCard } from '../UnsubscribeApprovalCard';

vi.mock('../../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

describe('UnsubscribeApprovalCard', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  const mockPayload = {
    status: 'pending_approval',
    action: 'unsubscribe',
    metadata: {
      sender: 'test@example.com',
      unsubscribe_link: 'https://example.com/unsub',
      message: 'Agent is requesting permission',
    },
  };

  it('renders correctly with pending payload', () => {
    renderWithProviders(<UnsubscribeApprovalCard payload={mockPayload} />);
    expect(screen.getByText('Unsubscribe Request')).toBeInTheDocument();
    expect(screen.getByText('Agent is requesting permission')).toBeInTheDocument();
    expect(screen.getByText('https://example.com/unsub')).toBeInTheDocument();
  });

  it('returns null if action is not unsubscribe', () => {
    const payload = { ...mockPayload, action: 'other' };
    const { container } = renderWithProviders(<UnsubscribeApprovalCard payload={payload} />);
    expect(container).toBeEmptyDOMElement();
  });

  it('returns null if status is not pending_approval', () => {
    const payload = { ...mockPayload, status: 'completed' };
    const { container } = renderWithProviders(<UnsubscribeApprovalCard payload={payload} />);
    expect(container).toBeEmptyDOMElement();
  });

  it('handles approval successfully', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ success: true });
    renderWithProviders(<UnsubscribeApprovalCard payload={mockPayload} />);

    const approveBtn = screen.getByText('Approve & Unsubscribe');
    fireEvent.click(approveBtn);

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'tools::execute_unsubscribe',
      params: { link: 'https://example.com/unsub' },
    });

    await waitFor(() => {
      expect(screen.getByText('✓ Successfully unsubscribed.')).toBeInTheDocument();
    });
  });

  it('handles denial', () => {
    renderWithProviders(<UnsubscribeApprovalCard payload={mockPayload} />);
    const denyBtn = screen.getByText('Deny');
    fireEvent.click(denyBtn);
    expect(screen.getByText('✕ Request denied.')).toBeInTheDocument();
  });

  it('displays error on RPC failure and missing permissions', async () => {
    vi.mocked(callCoreRpc).mockRejectedValueOnce(new Error('Missing Gmail write scopes'));
    renderWithProviders(<UnsubscribeApprovalCard payload={mockPayload} />);

    const approveBtn = screen.getByText('Approve & Unsubscribe');
    fireEvent.click(approveBtn);

    await waitFor(() => {
      expect(screen.getByText('⚠️ Missing Gmail write scopes')).toBeInTheDocument();
    });

    // Status should remain pending after error
    expect(screen.getByText('Approve & Unsubscribe')).toBeInTheDocument();
  });
});
