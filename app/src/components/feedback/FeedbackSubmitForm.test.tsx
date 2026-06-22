import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { CreateFeedbackResult, FeedbackItem } from '../../types/feedback';
import FeedbackSubmitForm from './FeedbackSubmitForm';

const mockSubmit = vi.fn();
vi.mock('../../services/api/feedbackApi', () => ({
  feedbackApi: { submitFeedback: (...args: unknown[]) => mockSubmit(...args) },
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

const accepted = (item: FeedbackItem): CreateFeedbackResult => ({
  accepted: true,
  reason: 'ok',
  feedback: item,
});

function fillForm(title: string, body: string) {
  fireEvent.change(screen.getByPlaceholderText('Title'), { target: { value: title } });
  fireEvent.change(screen.getByPlaceholderText('Describe your idea or the problem you hit'), {
    target: { value: body },
  });
}

describe('<FeedbackSubmitForm />', () => {
  beforeEach(() => mockSubmit.mockReset());

  it('exposes accessible labels for the title and body fields', () => {
    render(<FeedbackSubmitForm onAccepted={() => {}} />);
    expect(screen.getByRole('textbox', { name: 'Title' })).toBeInTheDocument();
    expect(
      screen.getByRole('textbox', { name: 'Describe your idea or the problem you hit' })
    ).toBeInTheDocument();
  });

  it('disables submit until both title and body are present', () => {
    render(<FeedbackSubmitForm onAccepted={() => {}} />);
    const submit = screen.getByRole('button', { name: 'Submit' });
    expect(submit).toBeDisabled();
    fillForm('A title', 'A body');
    expect(submit).toBeEnabled();
  });

  it('submits a trimmed feature payload, notifies the parent, clears, and shows success', async () => {
    const item = makeItem({ id: 'new', title: 'Dark mode' });
    mockSubmit.mockResolvedValueOnce(accepted(item));
    const onAccepted = vi.fn();

    render(<FeedbackSubmitForm onAccepted={onAccepted} />);
    fillForm('  Dark mode  ', '  please  ');
    fireEvent.click(screen.getByRole('button', { name: 'Submit' }));

    await waitFor(() =>
      expect(mockSubmit).toHaveBeenCalledWith({
        type: 'feature',
        title: 'Dark mode',
        body: 'please',
      })
    );
    expect(onAccepted).toHaveBeenCalledTimes(1);
    expect(
      await screen.findByText('Thanks! Your feedback is now on the board.')
    ).toBeInTheDocument();
    expect(screen.getByPlaceholderText('Title')).toHaveValue('');
  });

  it('sends type "bug" after toggling, and shows the moderation reason without notifying on reject', async () => {
    mockSubmit.mockResolvedValueOnce({
      accepted: false,
      reason: 'Looks like spam',
      feedback: null,
    });
    const onAccepted = vi.fn();

    render(<FeedbackSubmitForm onAccepted={onAccepted} />);
    fireEvent.click(screen.getByRole('button', { name: 'Bug' }));
    fillForm('Crash', 'it crashes');
    fireEvent.click(screen.getByRole('button', { name: 'Submit' }));

    await waitFor(() =>
      expect(mockSubmit).toHaveBeenCalledWith(expect.objectContaining({ type: 'bug' }))
    );
    expect(await screen.findByText('Looks like spam')).toBeInTheDocument();
    expect(onAccepted).not.toHaveBeenCalled();
  });

  it('surfaces an error when the request fails', async () => {
    mockSubmit.mockRejectedValueOnce(new Error('network down'));

    render(<FeedbackSubmitForm onAccepted={() => {}} />);
    fillForm('Title', 'Body');
    fireEvent.click(screen.getByRole('button', { name: 'Submit' }));

    expect(await screen.findByText('network down')).toBeInTheDocument();
  });
});
