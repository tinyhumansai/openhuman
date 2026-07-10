import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { ThreadGoal } from '../../../services/api/threadGoalApi';
import { ThreadGoalEditorPanel, ThreadGoalFooterTrigger, useThreadGoal } from './ThreadGoalChip';

// Echo i18n keys so assertions are on stable key strings.
vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

function goal(partial: Partial<ThreadGoal> = {}): ThreadGoal {
  return {
    threadId: 't1',
    goalId: 'g1',
    objective: 'Ship the release',
    status: 'active',
    tokenBudget: 1000,
    tokensUsed: 250,
    timeUsedSeconds: 0,
    createdAtMs: 0,
    updatedAtMs: 0,
    continuationSuppressed: false,
    ...partial,
  };
}

function stubApi(initial: ThreadGoal | null) {
  return {
    get: vi.fn().mockResolvedValue(initial),
    set: vi.fn().mockResolvedValue(goal()),
    complete: vi.fn().mockResolvedValue(goal({ status: 'complete' })),
    pause: vi.fn().mockResolvedValue(goal({ status: 'paused' })),
    resume: vi.fn().mockResolvedValue(goal()),
    clear: vi.fn().mockResolvedValue(true),
  } as unknown as typeof import('../../../services/api/threadGoalApi').threadGoalApi;
}

/** Harness wiring the shared controller to both pieces, as Conversations does. */
function Harness({ api }: { api: ReturnType<typeof stubApi> }) {
  const ctl = useThreadGoal('t1', api);
  return (
    <div>
      <ThreadGoalEditorPanel ctl={ctl} />
      <ThreadGoalFooterTrigger ctl={ctl} />
    </div>
  );
}

describe('ThreadGoal footer trigger + editor panel', () => {
  it('shows a footer "Set goal" trigger and no editor when empty', async () => {
    const api = stubApi(null);
    render(<Harness api={api} />);
    await waitFor(() => expect(api.get).toHaveBeenCalledWith('t1'));
    expect(await screen.findByText('conversations.threadGoal.setCta')).toBeTruthy();
    // Editor stays closed until clicked.
    expect(screen.queryByPlaceholderText('conversations.threadGoal.placeholder')).toBeNull();
  });

  it('clicking the trigger opens the editor input above; saving calls set', async () => {
    const api = stubApi(null);
    render(<Harness api={api} />);
    fireEvent.click(await screen.findByText('conversations.threadGoal.setCta'));
    const input = screen.getByPlaceholderText('conversations.threadGoal.placeholder');
    fireEvent.change(input, { target: { value: 'New objective' } });
    fireEvent.click(screen.getByText('conversations.threadGoal.save'));
    await waitFor(() => expect(api.set).toHaveBeenCalledWith('t1', 'New objective'));
  });

  it('shows the goal status + objective in the footer trigger when set', async () => {
    const api = stubApi(goal());
    render(<Harness api={api} />);
    expect(await screen.findByText('Ship the release')).toBeTruthy();
    expect(screen.getByText('conversations.threadGoal.status.active')).toBeTruthy();
    // Editor (with the token budget) is closed until the trigger is clicked.
    expect(screen.queryByText(/250 \/ 1,000/)).toBeNull();
  });

  it('opening an existing goal seeds the input and exposes lifecycle actions', async () => {
    const api = stubApi(goal());
    render(<Harness api={api} />);
    fireEvent.click(await screen.findByTitle('Ship the release'));
    const input = screen.getByPlaceholderText<HTMLInputElement>(
      'conversations.threadGoal.placeholder'
    );
    expect(input.value).toBe('Ship the release');
    expect(screen.getByText(/250 \/ 1,000/)).toBeTruthy();
    fireEvent.click(screen.getByLabelText('conversations.threadGoal.complete'));
    await waitFor(() => expect(api.complete).toHaveBeenCalledWith('t1'));
  });

  it('shows Resume (not Pause) in the editor for a paused goal', async () => {
    const api = stubApi(goal({ status: 'paused' }));
    render(<Harness api={api} />);
    fireEvent.click(await screen.findByTitle('Ship the release'));
    expect(screen.getByLabelText('conversations.threadGoal.resume')).toBeTruthy();
    expect(screen.queryByLabelText('conversations.threadGoal.pause')).toBeNull();
  });
});
