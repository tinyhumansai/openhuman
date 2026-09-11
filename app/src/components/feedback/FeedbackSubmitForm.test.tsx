import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { CreateFeedbackResult, FeedbackItem } from '../../types/feedback';
import FeedbackSubmitForm from './FeedbackSubmitForm';

const mockSubmit = vi.fn();
const mockValidate = vi.fn();
vi.mock('../../services/api/feedbackApi', () => ({
  feedbackApi: {
    submitFeedback: (...args: unknown[]) => mockSubmit(...args),
    validateFeedback: (...args: unknown[]) => mockValidate(...args),
  },
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

/** Let a resolved validate promise run its `.then` and flush the state update. */
async function settle() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

function fillForm(title: string, body: string) {
  fireEvent.change(screen.getByPlaceholderText('Title'), { target: { value: title } });
  fireEvent.change(screen.getByPlaceholderText('Describe your idea or the problem you hit'), {
    target: { value: body },
  });
}

describe('<FeedbackSubmitForm />', () => {
  beforeEach(() => {
    mockSubmit.mockReset();
    mockValidate.mockReset();
    mockValidate.mockResolvedValue({ tier: 'pass', reason: '' });
  });

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

describe('<FeedbackSubmitForm /> quality tiers', () => {
  beforeEach(() => {
    mockSubmit.mockReset();
    mockValidate.mockReset();
    mockValidate.mockResolvedValue({ tier: 'pass', reason: '' });
  });

  it('shows the reason while the draft is still being written', async () => {
    mockValidate.mockResolvedValue({ tier: 'warn', reason: 'Add steps to reproduce.' });

    render(<FeedbackSubmitForm onAccepted={() => {}} />);
    fillForm('Crash', 'It crashes.');

    const hint = await screen.findByTestId('feedback-quality-hint');
    expect(hint).toHaveTextContent('Add steps to reproduce.');
    expect(hint).toHaveAttribute('data-tier', 'warn');
  });

  // The point of checking as they type: stop the round trip that would be
  // refused anyway, while the text can still be changed.
  it('blocks submitting a draft the server would refuse', async () => {
    mockValidate.mockResolvedValue({ tier: 'block', reason: 'Please describe the problem.' });

    render(<FeedbackSubmitForm onAccepted={() => {}} />);
    fillForm('test', 'test');

    const hint = await screen.findByTestId('feedback-quality-hint');
    expect(hint).toHaveAttribute('data-tier', 'block');
    await waitFor(() => expect(screen.getByRole('button', { name: 'Submit' })).toBeDisabled());

    fireEvent.click(screen.getByRole('button', { name: 'Submit' }));
    expect(mockSubmit).not.toHaveBeenCalled();
  });

  it('says nothing about a draft that passes', async () => {
    render(<FeedbackSubmitForm onAccepted={() => {}} />);
    fillForm('A real title', 'A real description of the problem.');

    await waitFor(() => expect(mockValidate).toHaveBeenCalled());
    expect(screen.queryByTestId('feedback-quality-hint')).not.toBeInTheDocument();
  });

  it('does not ask the server about an empty draft', () => {
    // A real-timer wait would have to outlive the debounce window to prove
    // nothing fires — a time flake on a slow or loaded CI node. Fake timers
    // make the proof deterministic.
    vi.useFakeTimers();
    try {
      render(<FeedbackSubmitForm onAccepted={() => {}} />);
      fillForm('', '');

      // Far past the debounce window: if an empty draft were ever scheduled,
      // this would fire the call and the assertion below would fail.
      act(() => {
        vi.advanceTimersByTime(1000);
      });
      expect(mockValidate).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });

  // Warn is advisory. The submission is published; the reason is a nudge.
  it('still publishes a warned submission and keeps its reason visible', async () => {
    mockValidate.mockResolvedValue({ tier: 'warn', reason: 'Add steps to reproduce.' });
    mockSubmit.mockResolvedValueOnce({
      accepted: true,
      reason: 'ok',
      feedback: makeItem(),
      quality: { tier: 'warn', reason: 'Add steps to reproduce.' },
    });
    const onAccepted = vi.fn();

    render(<FeedbackSubmitForm onAccepted={onAccepted} />);
    fillForm('Crash', 'It crashes.');
    await screen.findByTestId('feedback-quality-hint');
    fireEvent.click(screen.getByRole('button', { name: 'Submit' }));

    await waitFor(() => expect(onAccepted).toHaveBeenCalled());
    expect(await screen.findByText('Add steps to reproduce.')).toBeInTheDocument();
  });

  // The advice from an accepted-with-warning submission outlives the draft it
  // was about, so the only thing that should discard it is an edit. Selecting
  // the type you are already on is not an edit -- and it went from an unlikely
  // misclick on a full-width button to an easy one on an adjacent pill.
  it('keeps warned-submission advice when the already-selected type is re-clicked', async () => {
    mockValidate.mockResolvedValue({ tier: 'warn', reason: 'Add steps to reproduce.' });
    mockSubmit.mockResolvedValueOnce({
      accepted: true,
      reason: 'ok',
      feedback: makeItem(),
      quality: { tier: 'warn', reason: 'Add steps to reproduce.' },
    });

    render(<FeedbackSubmitForm onAccepted={() => {}} />);
    fillForm('Crash', 'It crashes.');
    await screen.findByTestId('feedback-quality-hint');
    fireEvent.click(screen.getByRole('button', { name: 'Submit' }));
    expect(await screen.findByText('Add steps to reproduce.')).toBeInTheDocument();

    // `feature` is the default, so this is the pill that is already active.
    fireEvent.click(screen.getByRole('button', { name: 'Feature' }));

    expect(screen.getByText('Add steps to reproduce.')).toBeInTheDocument();
  });

  // The other half of the same guard: it must skip the reset only when the type
  // is unchanged. A guard that swallowed the reset outright would leave stale
  // advice attached to a draft whose type just changed.
  it('discards warned-submission advice when the type actually changes', async () => {
    mockValidate.mockResolvedValue({ tier: 'warn', reason: 'Add steps to reproduce.' });
    mockSubmit.mockResolvedValueOnce({
      accepted: true,
      reason: 'ok',
      feedback: makeItem(),
      quality: { tier: 'warn', reason: 'Add steps to reproduce.' },
    });

    render(<FeedbackSubmitForm onAccepted={() => {}} />);
    fillForm('Crash', 'It crashes.');
    await screen.findByTestId('feedback-quality-hint');
    fireEvent.click(screen.getByRole('button', { name: 'Submit' }));
    expect(await screen.findByText('Add steps to reproduce.')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Bug' }));

    await waitFor(() =>
      expect(screen.queryByText('Add steps to reproduce.')).not.toBeInTheDocument()
    );
  });

  // apiClient rejects with a plain `{ success, error }` object, not an Error.
  // Without handling that shape the server's reason is replaced by the generic
  // failure copy, and a blocked submitter is told nothing useful.
  it('surfaces the server reason when only the server catches the block', async () => {
    mockSubmit.mockRejectedValueOnce({ success: false, error: 'You have already reported this.' });

    render(<FeedbackSubmitForm onAccepted={() => {}} />);
    fillForm('Crash', 'It crashes.');
    fireEvent.click(screen.getByRole('button', { name: 'Submit' }));

    expect(await screen.findByText('You have already reported this.')).toBeInTheDocument();
  });

  // A quality verdict must not read as a moderation flag.
  it('keeps the quality hint separate from a moderation rejection', async () => {
    mockSubmit.mockResolvedValueOnce({
      accepted: false,
      reason: 'Looks like spam',
      feedback: null,
    });

    render(<FeedbackSubmitForm onAccepted={() => {}} />);
    fillForm('Crash', 'It crashes.');
    fireEvent.click(screen.getByRole('button', { name: 'Submit' }));

    expect(await screen.findByText('Looks like spam')).toBeInTheDocument();
    expect(screen.queryByTestId('feedback-quality-hint')).not.toBeInTheDocument();
  });

  // The verdict is keyed to the draft it was computed for. Without that, a slow
  // `block` for text the user has already replaced would arrive and disable
  // submit for a draft it was never about.
  //
  // The second check is left unresolved on purpose: if it answered, its verdict
  // would replace the stale one and the test would pass whether or not the
  // keying exists.
  it('ignores a verdict that arrives for a draft the user has since changed', async () => {
    let resolveFirst!: (value: { tier: string; reason: string }) => void;
    mockValidate
      .mockImplementationOnce(
        () =>
          new Promise(resolve => {
            resolveFirst = resolve;
          })
      )
      .mockImplementationOnce(() => new Promise(() => {}));

    render(<FeedbackSubmitForm onAccepted={() => {}} />);
    fillForm('test', 'test');
    await waitFor(() => expect(mockValidate).toHaveBeenCalledTimes(1));

    // The user rewrites it into a real report before the first verdict lands.
    fillForm('Upload fails on retry', 'Uploading a second file after a failure hangs forever.');
    await waitFor(() => expect(mockValidate).toHaveBeenCalledTimes(2));

    resolveFirst({ tier: 'block', reason: 'Please describe the problem.' });
    await waitFor(() => expect(screen.getByRole('button', { name: 'Submit' })).toBeEnabled());
    expect(screen.queryByText('Please describe the problem.')).not.toBeInTheDocument();
    expect(screen.queryByTestId('feedback-quality-hint')).not.toBeInTheDocument();
  });

  // The sibling of the test above, and the case the draft key alone does not
  // cover: the stale call answers *after* the current one, so keying the read
  // is not enough — the late write has to be dropped, or it replaces a correct
  // verdict with one that no longer matches the draft and the hint vanishes.
  it('keeps the current verdict when a superseded check answers late', async () => {
    let resolveStale!: (value: { tier: string; reason: string }) => void;
    let resolveCurrent!: (value: { tier: string; reason: string }) => void;
    mockValidate
      .mockImplementationOnce(
        () =>
          new Promise(resolve => {
            resolveStale = resolve;
          })
      )
      .mockImplementationOnce(
        () =>
          new Promise(resolve => {
            resolveCurrent = resolve;
          })
      );

    render(<FeedbackSubmitForm onAccepted={() => {}} />);
    fillForm('test', 'test');
    await waitFor(() => expect(mockValidate).toHaveBeenCalledTimes(1));

    fillForm('Upload fails on retry', 'Uploading a second file after a failure hangs forever.');
    await waitFor(() => expect(mockValidate).toHaveBeenCalledTimes(2));

    resolveCurrent({ tier: 'warn', reason: 'Add steps to reproduce.' });
    expect(await screen.findByTestId('feedback-quality-hint')).toHaveTextContent(
      'Add steps to reproduce.'
    );

    resolveStale({ tier: 'block', reason: 'Please describe the problem.' });
    await settle();

    expect(screen.getByTestId('feedback-quality-hint')).toHaveTextContent(
      'Add steps to reproduce.'
    );
    expect(screen.getByRole('button', { name: 'Submit' })).toBeEnabled();
  });

  // A disabled submit with nothing on screen is a dead end. If the gate cannot
  // say why, let the submit through and take the server's refusal, which can.
  it('does not disable submit for a block it cannot explain', async () => {
    mockValidate.mockResolvedValue({ tier: 'block', reason: '' });

    render(<FeedbackSubmitForm onAccepted={() => {}} />);
    fillForm('test', 'test');

    await waitFor(() => expect(mockValidate).toHaveBeenCalled());
    await settle();

    expect(screen.queryByTestId('feedback-quality-hint')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Submit' })).toBeEnabled();
  });

  // The hint appears ~300ms after typing stops and is the only account of why
  // submit went disabled. Nothing moves focus to it, so it has to announce —
  // and the region has to be in the tree *before* the text lands, because a
  // region inserted along with its content gives AT no change to observe.
  it('announces the hint from a live region that predates it, and describes submit with it', async () => {
    mockValidate.mockResolvedValue({ tier: 'block', reason: 'Please describe the problem.' });

    render(<FeedbackSubmitForm onAccepted={() => {}} />);
    const region = screen.getByRole('status');
    expect(region).toHaveAttribute('aria-live', 'polite');
    expect(region).toBeEmptyDOMElement();

    fillForm('test', 'test');

    const hint = await screen.findByTestId('feedback-quality-hint');
    expect(region).toContainElement(hint);
    expect(hint.id).toBe('feedback-quality-hint');
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Submit' })).toHaveAttribute(
        'aria-describedby',
        'feedback-quality-hint'
      )
    );
  });

  // Changing the type is an edit like any other, so the advice the last
  // submission came back with is no longer about what is on screen. Both
  // directions, because each toggle has its own handler.
  it.each([
    ['Feature', 'Bug'],
    ['Bug', 'Feature'],
  ])('drops the last submission advice when the type changes from %s to %s', async (from, to) => {
    mockValidate.mockResolvedValue({ tier: 'warn', reason: 'Add steps to reproduce.' });
    mockSubmit.mockResolvedValueOnce({
      accepted: true,
      reason: 'ok',
      feedback: makeItem(),
      quality: { tier: 'warn', reason: 'Add steps to reproduce.' },
    });

    render(<FeedbackSubmitForm onAccepted={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: from }));
    fillForm('Crash', 'It crashes.');
    await screen.findByTestId('feedback-quality-hint');
    fireEvent.click(screen.getByRole('button', { name: 'Submit' }));

    // The form clears but the advice outlives the text it was about.
    await waitFor(() => expect(screen.getByPlaceholderText('Title')).toHaveValue(''));
    expect(screen.getByTestId('feedback-quality-hint')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: to }));
    expect(screen.queryByTestId('feedback-quality-hint')).not.toBeInTheDocument();
  });
});
