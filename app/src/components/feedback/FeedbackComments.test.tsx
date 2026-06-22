import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import type { FeedbackComment } from '../../types/feedback';
import FeedbackComments from './FeedbackComments';

const mockGetFeedback = vi.fn();
const mockAddComment = vi.fn();
vi.mock('../../services/api/feedbackApi', () => ({
  feedbackApi: {
    getFeedback: (...args: unknown[]) => mockGetFeedback(...args),
    addComment: (...args: unknown[]) => mockAddComment(...args),
  },
}));

function makeComment(overrides: Partial<FeedbackComment> = {}): FeedbackComment {
  return {
    id: 'c1',
    user: 'user-abc123',
    userName: null,
    body: 'first comment',
    createdAt: '2026-01-01T00:00:00.000Z',
    ...overrides,
  };
}

describe('<FeedbackComments />', () => {
  beforeEach(() => {
    mockGetFeedback.mockReset();
    mockAddComment.mockReset();
  });

  it('loads and renders existing comments', async () => {
    mockGetFeedback.mockResolvedValueOnce({ feedback: { id: 'f1' }, comments: [makeComment()] });

    renderWithProviders(<FeedbackComments feedbackId="f1" onCommentAdded={() => {}} />);

    expect(await screen.findByText('first comment')).toBeInTheDocument();
    expect(mockGetFeedback).toHaveBeenCalledWith('f1');
  });

  it('shows the empty state when there are no comments', async () => {
    mockGetFeedback.mockResolvedValueOnce({ feedback: { id: 'f1' }, comments: [] });

    renderWithProviders(<FeedbackComments feedbackId="f1" onCommentAdded={() => {}} />);

    expect(await screen.findByText('No comments yet.')).toBeInTheDocument();
  });

  it('posts a comment, appends it, and notifies the parent', async () => {
    mockGetFeedback.mockResolvedValueOnce({ feedback: { id: 'f1' }, comments: [] });
    mockAddComment.mockResolvedValueOnce(makeComment({ id: 'c2', body: 'a new comment' }));
    const onCommentAdded = vi.fn();

    renderWithProviders(<FeedbackComments feedbackId="f1" onCommentAdded={onCommentAdded} />);
    await screen.findByText('No comments yet.');

    fireEvent.change(screen.getByPlaceholderText('Add a comment'), {
      target: { value: 'a new comment' },
    });
    fireEvent.click(screen.getByText('Post'));

    await waitFor(() => expect(mockAddComment).toHaveBeenCalledWith('f1', 'a new comment'));
    expect(await screen.findByText('a new comment')).toBeInTheDocument();
    expect(onCommentAdded).toHaveBeenCalledTimes(1);
  });

  it('surfaces a load error', async () => {
    mockGetFeedback.mockRejectedValueOnce(new Error('nope'));

    renderWithProviders(<FeedbackComments feedbackId="f1" onCommentAdded={() => {}} />);

    expect(await screen.findByText('nope')).toBeInTheDocument();
  });
});
