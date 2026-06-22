import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { FeedbackItem } from '../../types/feedback';
import FeedbackVoteControl from './FeedbackVoteControl';

const mockVote = vi.fn();
vi.mock('../../services/api/feedbackApi', () => ({
  feedbackApi: { voteFeedback: (...args: unknown[]) => mockVote(...args) },
}));

function makeItem(overrides: Partial<FeedbackItem> = {}): FeedbackItem {
  return {
    id: 'f1',
    type: 'feature',
    title: 'Title',
    body: 'Body',
    status: 'open',
    createdBy: 'u1',
    createdByName: null,
    upvoteCount: 3,
    downvoteCount: 1,
    score: 2,
    rankScore: 0,
    commentCount: 0,
    github: null,
    myVote: 0,
    createdAt: '2026-01-01T00:00:00.000Z',
    updatedAt: '2026-01-01T00:00:00.000Z',
    ...overrides,
  };
}

describe('<FeedbackVoteControl />', () => {
  beforeEach(() => mockVote.mockReset());

  it('optimistically applies an upvote, then reconciles with the server result', async () => {
    const item = makeItem();
    const server = makeItem({ upvoteCount: 4, score: 3, myVote: 1 });
    mockVote.mockResolvedValueOnce(server);
    const onVoted = vi.fn();

    render(<FeedbackVoteControl item={item} onVoted={onVoted} />);
    fireEvent.click(screen.getByLabelText('Upvote'));

    // First call is the optimistic update (score 2 -> 3, myVote 1).
    expect(onVoted).toHaveBeenNthCalledWith(
      1,
      expect.objectContaining({ score: 3, upvoteCount: 4, myVote: 1 })
    );
    expect(mockVote).toHaveBeenCalledWith('f1', 1);
    // Second call is the authoritative server item.
    await waitFor(() => expect(onVoted).toHaveBeenNthCalledWith(2, server));
  });

  it('retracts the vote when the active direction is clicked again', async () => {
    const item = makeItem({ myVote: 1, upvoteCount: 4, score: 3 });
    mockVote.mockResolvedValueOnce(makeItem({ myVote: 0 }));
    const onVoted = vi.fn();

    render(<FeedbackVoteControl item={item} onVoted={onVoted} />);
    fireEvent.click(screen.getByLabelText('Upvote'));

    expect(mockVote).toHaveBeenCalledWith('f1', 0);
    expect(onVoted).toHaveBeenNthCalledWith(
      1,
      expect.objectContaining({ score: 2, upvoteCount: 3, myVote: 0 })
    );
  });

  it('rolls back to the previous item when the vote request fails', async () => {
    const item = makeItem();
    mockVote.mockRejectedValueOnce(new Error('network'));
    const onVoted = vi.fn();

    render(<FeedbackVoteControl item={item} onVoted={onVoted} />);
    fireEvent.click(screen.getByLabelText('Downvote'));

    // Optimistic first, then rollback to the original item.
    expect(onVoted).toHaveBeenNthCalledWith(
      1,
      expect.objectContaining({ score: 1, downvoteCount: 2, myVote: -1 })
    );
    await waitFor(() => expect(onVoted).toHaveBeenNthCalledWith(2, item));
  });
});
