import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../test/test-utils';
import type { FeedbackItem } from '../types/feedback';
import Feedback, { acceptedItemMatchesFilters } from './Feedback';

const mockList = vi.fn();
const mockVote = vi.fn();
const mockSubmit = vi.fn();
const mockUpdateStatus = vi.fn();
const mockGetFeedback = vi.fn();
const mockAddComment = vi.fn();

vi.mock('../services/api/feedbackApi', () => ({
  feedbackApi: {
    listFeedback: (...args: unknown[]) => mockList(...args),
    voteFeedback: (...args: unknown[]) => mockVote(...args),
    submitFeedback: (...args: unknown[]) => mockSubmit(...args),
    updateStatus: (...args: unknown[]) => mockUpdateStatus(...args),
    getFeedback: (...args: unknown[]) => mockGetFeedback(...args),
    addComment: (...args: unknown[]) => mockAddComment(...args),
  },
}));

// Drive `isAdmin` (which gates the status control) without standing up a full
// core-state snapshot; flip `role` to 'admin' only in the tests that need it.
const userRole = vi.hoisted(() => ({ current: 'user' as 'user' | 'admin' }));
vi.mock('../hooks/useUser', () => ({
  useUser: () => ({
    user: { role: userRole.current },
    isLoading: false,
    error: null,
    refetch: () => {},
  }),
}));

function makeItem(overrides: Partial<FeedbackItem> = {}): FeedbackItem {
  return {
    id: 'f1',
    type: 'feature',
    title: 'Add dark mode',
    body: 'Please add a dark theme',
    status: 'open',
    createdBy: 'u1',
    createdByName: null,
    upvoteCount: 5,
    downvoteCount: 0,
    score: 5,
    rankScore: 0,
    commentCount: 0,
    github: null,
    myVote: 0,
    createdAt: '2026-01-01T00:00:00.000Z',
    updatedAt: '2026-01-01T00:00:00.000Z',
    ...overrides,
  };
}

describe('<Feedback />', () => {
  beforeEach(() => {
    mockList.mockReset();
    mockVote.mockReset();
    mockSubmit.mockReset();
    mockUpdateStatus.mockReset();
    mockGetFeedback.mockReset();
    mockAddComment.mockReset();
    userRole.current = 'user';
  });

  it('loads and renders feedback items from the board', async () => {
    mockList.mockResolvedValueOnce({ items: [makeItem()], total: 1, page: 1, limit: 20 });

    renderWithProviders(<Feedback />);

    expect(await screen.findByText('Add dark mode')).toBeInTheDocument();
    expect(mockList).toHaveBeenCalledWith(
      expect.objectContaining({ sort: 'hot', page: 1, limit: 20 })
    );
    // The submit form is always present.
    expect(screen.getByText('Share feedback')).toBeInTheDocument();
  });

  it('shows the empty state when there is no feedback', async () => {
    mockList.mockResolvedValueOnce({ items: [], total: 0, page: 1, limit: 20 });

    renderWithProviders(<Feedback />);

    expect(
      await screen.findByText('No feedback yet. Be the first to share an idea.')
    ).toBeInTheDocument();
  });

  it('surfaces a load error without also showing the empty state', async () => {
    mockList.mockRejectedValueOnce(new Error('boom'));

    renderWithProviders(<Feedback />);

    await waitFor(() => expect(screen.getByText('boom')).toBeInTheDocument());
    // The empty-state copy must not render alongside the error banner.
    expect(
      screen.queryByText('No feedback yet. Be the first to share an idea.')
    ).not.toBeInTheDocument();
  });
});

describe('<Feedback /> keeps the board in sync after local mutations', () => {
  beforeEach(() => {
    mockList.mockReset();
    mockVote.mockReset();
    mockSubmit.mockReset();
    mockUpdateStatus.mockReset();
    mockGetFeedback.mockReset();
    mockAddComment.mockReset();
    userRole.current = 'user';
  });

  async function openFilter(
    user: ReturnType<typeof userEvent.setup>,
    triggerLabel: string,
    optionName: string
  ) {
    await user.click(screen.getByLabelText(triggerLabel));
    const listbox = await screen.findByRole('listbox');
    await user.click(within(listbox).getByRole('button', { name: optionName }));
  }

  async function submitFeature(user: ReturnType<typeof userEvent.setup>, title: string) {
    await user.type(screen.getByPlaceholderText('Title'), title);
    await user.type(
      screen.getByPlaceholderText('Describe your idea or the problem you hit'),
      'Some supporting detail'
    );
    await user.click(screen.getByRole('button', { name: 'Submit' }));
  }

  it('reloads from page 1 when an accepted submission matches the active filters', async () => {
    mockList
      .mockResolvedValueOnce({
        items: [makeItem({ id: 'f1', title: 'Existing' })],
        total: 1,
        page: 1,
        limit: 20,
      })
      .mockResolvedValueOnce({
        items: [
          makeItem({ id: 'f2', title: 'Brand new' }),
          makeItem({ id: 'f1', title: 'Existing' }),
        ],
        total: 2,
        page: 1,
        limit: 20,
      });
    mockSubmit.mockResolvedValueOnce({
      accepted: true,
      feedback: makeItem({ id: 'f2', title: 'Brand new', type: 'feature', status: 'open' }),
      reason: null,
    });

    const user = userEvent.setup();
    renderWithProviders(<Feedback />);
    await screen.findByText('Existing');

    await submitFeature(user, 'Brand new');

    // The new item is shown via a fresh page-1 fetch, not an optimistic prepend.
    expect(await screen.findByText('Brand new')).toBeInTheDocument();
    expect(mockList).toHaveBeenCalledTimes(2);
    expect(mockList).toHaveBeenLastCalledWith(expect.objectContaining({ page: 1 }));
  });

  it('does not refetch when an accepted submission falls outside the active filter', async () => {
    mockList
      .mockResolvedValueOnce({
        items: [makeItem({ id: 'b1', type: 'bug', title: 'A bug' })],
        total: 1,
        page: 1,
        limit: 20,
      })
      .mockResolvedValueOnce({
        items: [makeItem({ id: 'b1', type: 'bug', title: 'A bug' })],
        total: 1,
        page: 1,
        limit: 20,
      });
    mockSubmit.mockResolvedValueOnce({
      accepted: true,
      feedback: makeItem({ id: 'f9', type: 'feature', status: 'open', title: 'New feature idea' }),
      reason: null,
    });

    const user = userEvent.setup();
    renderWithProviders(<Feedback />);
    await screen.findByText('A bug');

    await openFilter(user, 'All types', 'Bug');
    await waitFor(() => expect(mockList).toHaveBeenCalledTimes(2));

    await submitFeature(user, 'New feature idea');
    await waitFor(() => expect(mockSubmit).toHaveBeenCalled());

    // The feature can't belong to a Bugs view, so no extra fetch and it never appears.
    expect(mockList).toHaveBeenCalledTimes(2);
    expect(screen.queryByText('New feature idea')).not.toBeInTheDocument();
  });

  it('patches a row in place on vote without refetching the board', async () => {
    mockList.mockResolvedValueOnce({
      items: [makeItem({ id: 'f1', title: 'Votable', score: 5, upvoteCount: 5, myVote: 0 })],
      total: 1,
      page: 1,
      limit: 20,
    });
    mockVote.mockResolvedValueOnce(
      makeItem({ id: 'f1', title: 'Votable', score: 6, upvoteCount: 6, myVote: 1 })
    );

    const user = userEvent.setup();
    renderWithProviders(<Feedback />);
    await screen.findByText('Votable');

    await user.click(screen.getByRole('button', { name: 'Upvote' }));

    expect(await screen.findByText('6')).toBeInTheDocument();
    expect(mockList).toHaveBeenCalledTimes(1);
  });

  it('merges a comment-count bump in place without refetching the board', async () => {
    mockList.mockResolvedValueOnce({
      items: [makeItem({ id: 'f1', title: 'Commentable', commentCount: 2 })],
      total: 1,
      page: 1,
      limit: 20,
    });
    mockGetFeedback.mockResolvedValueOnce({ feedback: makeItem({ id: 'f1' }), comments: [] });
    mockAddComment.mockResolvedValueOnce({
      id: 'c1',
      user: 'u2',
      userName: null,
      body: 'Great call',
      createdAt: '2026-01-01T00:00:00.000Z',
    });

    const user = userEvent.setup();
    renderWithProviders(<Feedback />);
    await screen.findByText('Commentable');

    await user.click(screen.getByText('Show more'));
    await user.type(await screen.findByPlaceholderText('Add a comment'), 'Great call');
    await user.click(screen.getByRole('button', { name: 'Post' }));

    // The posted comment renders and the count merges locally — no board refetch.
    expect(await screen.findByText('Great call')).toBeInTheDocument();
    expect(mockList).toHaveBeenCalledTimes(1);
  });

  it('drops a row and reloads when an admin status change leaves the active filter', async () => {
    userRole.current = 'admin';
    const openItem = makeItem({ id: 'f1', title: 'Open item', status: 'open' });
    mockList
      .mockResolvedValueOnce({ items: [openItem], total: 1, page: 1, limit: 20 }) // initial (all statuses)
      .mockResolvedValueOnce({ items: [openItem], total: 1, page: 1, limit: 20 }) // filtered to Open
      .mockResolvedValueOnce({ items: [], total: 0, page: 1, limit: 20 }); // reload after it leaves the filter
    mockUpdateStatus.mockResolvedValueOnce(
      makeItem({ id: 'f1', title: 'Open item', status: 'completed' })
    );

    const user = userEvent.setup();
    renderWithProviders(<Feedback />);
    await screen.findByText('Open item');

    await openFilter(user, 'All statuses', 'Open');
    await waitFor(() => expect(mockList).toHaveBeenCalledTimes(2));

    await user.selectOptions(screen.getByRole('combobox'), 'completed');

    await waitFor(() => expect(mockUpdateStatus).toHaveBeenCalledWith('f1', 'completed'));
    // The completed row no longer matches the Open filter: reload drops it.
    await waitFor(() => expect(mockList).toHaveBeenCalledTimes(3));
    await waitFor(() => expect(screen.queryByText('Open item')).not.toBeInTheDocument());
  });
});

describe('acceptedItemMatchesFilters', () => {
  const feature = makeItem({ type: 'feature', status: 'open' });

  it('matches when filters are "all"', () => {
    expect(acceptedItemMatchesFilters(feature, 'all', 'all')).toBe(true);
  });

  it('excludes an item whose type does not match the active type filter', () => {
    expect(acceptedItemMatchesFilters(feature, 'bug', 'all')).toBe(false);
  });

  it('excludes an item whose status does not match the active status filter', () => {
    // New submissions are always "open", so a board filtered to "completed" must not show them.
    expect(acceptedItemMatchesFilters(feature, 'all', 'completed')).toBe(false);
  });

  it('includes an item that matches both active filters', () => {
    expect(acceptedItemMatchesFilters(feature, 'feature', 'open')).toBe(true);
  });
});
