import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { FeedbackItem } from '../../types/feedback';
import FeedbackAdminMenu from './FeedbackAdminMenu';

const mockUpdateStatus = vi.fn();
vi.mock('../../services/api/feedbackApi', () => ({
  feedbackApi: { updateStatus: (...args: unknown[]) => mockUpdateStatus(...args) },
}));

function makeItem(overrides: Partial<FeedbackItem> = {}): FeedbackItem {
  return {
    id: 'f1',
    type: 'feature',
    title: 'T',
    body: 'B',
    status: 'open',
    createdBy: 'u1',
    createdByName: null,
    upvoteCount: 0,
    downvoteCount: 0,
    score: 0,
    rankScore: 0,
    commentCount: 0,
    github: null,
    myVote: 0,
    createdAt: '2026-01-01T00:00:00.000Z',
    updatedAt: '2026-01-01T00:00:00.000Z',
    ...overrides,
  };
}

describe('<FeedbackAdminMenu />', () => {
  beforeEach(() => mockUpdateStatus.mockReset());

  it('shows the current status in the control', () => {
    render(<FeedbackAdminMenu item={makeItem({ status: 'planned' })} onUpdated={() => {}} />);
    expect(screen.getByRole('combobox')).toHaveValue('planned');
  });

  it('associates the status label with the select for assistive tech', () => {
    render(<FeedbackAdminMenu item={makeItem({ status: 'open' })} onUpdated={() => {}} />);
    expect(screen.getByRole('combobox', { name: 'Status' })).toBeInTheDocument();
  });

  it('updates the status and bubbles the result', async () => {
    const updated = makeItem({ status: 'completed' });
    mockUpdateStatus.mockResolvedValueOnce(updated);
    const onUpdated = vi.fn();

    render(<FeedbackAdminMenu item={makeItem({ status: 'open' })} onUpdated={onUpdated} />);
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'completed' } });

    await waitFor(() => expect(mockUpdateStatus).toHaveBeenCalledWith('f1', 'completed'));
    expect(onUpdated).toHaveBeenCalledWith(updated);
  });

  it('surfaces an error when the update fails', async () => {
    mockUpdateStatus.mockRejectedValueOnce(new Error('forbidden'));

    render(<FeedbackAdminMenu item={makeItem({ status: 'open' })} onUpdated={() => {}} />);
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'planned' } });

    expect(await screen.findByText('forbidden')).toBeInTheDocument();
  });
});
