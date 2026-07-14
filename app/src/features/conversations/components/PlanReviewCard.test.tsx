import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { PendingPlanReview } from '../../../store/chatRuntimeSlice';
import { PlanReviewCard } from './PlanReviewCard';

// Echo i18n keys so we can assert on the stable key string.
vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

const mockCallCoreRpc = vi.fn();
vi.mock('../../../services/coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mockCallCoreRpc(...args),
}));

const mockDispatch = vi.fn();
vi.mock('../../../store/hooks', () => ({ useAppDispatch: () => mockDispatch }));

function review(partial: Partial<PendingPlanReview> = {}): PendingPlanReview {
  return {
    requestId: 'r1',
    summary: 'Ship the release',
    steps: ['step one', 'step two'],
    ...partial,
  };
}

describe('PlanReviewCard', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset().mockResolvedValue({});
    mockDispatch.mockReset();
  });

  it('renders the summary and ordered steps', () => {
    render(<PlanReviewCard threadId="t1" review={review()} />);
    expect(screen.getByText('Ship the release')).toBeInTheDocument();
    expect(screen.getByText('step one')).toBeInTheDocument();
    expect(screen.getByText('step two')).toBeInTheDocument();
  });

  it('approves via plan_review_decide and clears optimistically', async () => {
    render(<PlanReviewCard threadId="t1" review={review()} />);
    fireEvent.click(screen.getByText('conversations.planReview.approve'));
    await waitFor(() =>
      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.plan_review_decide',
        params: { request_id: 'r1', decision: 'approve', feedback: undefined },
      })
    );
    expect(mockDispatch).toHaveBeenCalledTimes(1);
  });

  it('rejects via plan_review_decide', async () => {
    render(<PlanReviewCard threadId="t1" review={review()} />);
    fireEvent.click(screen.getByText('conversations.planReview.reject'));
    await waitFor(() =>
      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.plan_review_decide',
        params: { request_id: 'r1', decision: 'reject', feedback: undefined },
      })
    );
  });

  it('sends trimmed feedback as a revise decision; ignores blank input', async () => {
    render(<PlanReviewCard threadId="t1" review={review()} />);
    const send = screen.getByText('conversations.planReview.sendFeedback');
    const textarea = screen.getByTestId('plan-review-feedback') as HTMLTextAreaElement;

    // Blank → disabled, no call.
    fireEvent.click(send);
    expect(mockCallCoreRpc).not.toHaveBeenCalled();

    fireEvent.change(textarea, { target: { value: '  add a verification step  ' } });
    fireEvent.click(send);
    await waitFor(() =>
      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.plan_review_decide',
        params: { request_id: 'r1', decision: 'revise', feedback: 'add a verification step' },
      })
    );
  });

  it('surfaces an error and stays mounted when the RPC fails', async () => {
    mockCallCoreRpc.mockRejectedValueOnce(new Error('boom'));
    render(<PlanReviewCard threadId="t1" review={review()} />);
    fireEvent.click(screen.getByText('conversations.planReview.approve'));
    await waitFor(() => expect(screen.getByText(/chat\.approval\.error/)).toBeInTheDocument());
    // Not cleared on failure.
    expect(mockDispatch).not.toHaveBeenCalled();
  });
});
