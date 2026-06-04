import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { todosApi, USER_TASKS_THREAD_ID } from '../../services/api/todosApi';
import { UserTaskComposer } from './UserTaskComposer';

vi.mock('../../store/hooks', () => ({
  useAppSelector: (sel: (state: unknown) => unknown) =>
    sel({
      thread: {
        threads: [
          { id: 't-1', title: 'Plan trip' },
          { id: 'worker-1', title: 'Worker', parentThreadId: 't-1' },
        ],
      },
    }),
}));

vi.mock('../../services/api/todosApi', () => ({
  USER_TASKS_THREAD_ID: 'user-tasks',
  todosApi: { add: vi.fn(), edit: vi.fn() },
}));

const mockAdd = vi.mocked(todosApi.add);
const mockEdit = vi.mocked(todosApi.edit);

function emptyBoard(threadId: string) {
  return { threadId, cards: [], updatedAt: '' };
}

describe('UserTaskComposer', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('disables Create until a title is entered', () => {
    render(<UserTaskComposer onCreated={vi.fn()} onClose={vi.fn()} />);
    const createBtn = screen.getByRole('button', { name: 'Create task' });
    expect(createBtn).toBeDisabled();
    fireEvent.change(screen.getByPlaceholderText('What needs to be done?'), {
      target: { value: 'Buy milk' },
    });
    expect(createBtn).toBeEnabled();
  });

  it('creates a task on the personal board by default', async () => {
    mockAdd.mockResolvedValueOnce(emptyBoard(USER_TASKS_THREAD_ID));
    const onCreated = vi.fn();
    const onClose = vi.fn();
    render(<UserTaskComposer onCreated={onCreated} onClose={onClose} />);

    fireEvent.change(screen.getByPlaceholderText('What needs to be done?'), {
      target: { value: 'Buy milk' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Create task' }));

    await waitFor(() => expect(mockAdd).toHaveBeenCalledTimes(1));
    expect(mockAdd).toHaveBeenCalledWith({
      threadId: USER_TASKS_THREAD_ID,
      content: 'Buy milk',
      status: 'todo',
      objective: null,
      notes: null,
    });
    expect(onCreated).toHaveBeenCalledWith(USER_TASKS_THREAD_ID, expect.any(Object));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('attaches the task to a chosen conversation', async () => {
    mockAdd.mockResolvedValueOnce(emptyBoard('t-1'));
    render(<UserTaskComposer onCreated={vi.fn()} onClose={vi.fn()} />);

    fireEvent.change(screen.getByPlaceholderText('What needs to be done?'), {
      target: { value: 'Book hotel' },
    });
    // The attach selector lists user-initiated threads (worker threads excluded).
    expect(screen.queryByRole('option', { name: 'Worker' })).not.toBeInTheDocument();
    fireEvent.change(screen.getByDisplayValue('Personal (no conversation)'), {
      target: { value: 't-1' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Create task' }));

    await waitFor(() => expect(mockAdd).toHaveBeenCalledTimes(1));
    expect(mockAdd.mock.calls[0][0].threadId).toBe('t-1');
  });

  it('assigns the new card to the orchestrator atomically when "assign to agent" is on', async () => {
    mockAdd.mockResolvedValueOnce(emptyBoard(USER_TASKS_THREAD_ID));
    render(<UserTaskComposer onCreated={vi.fn()} onClose={vi.fn()} />);

    fireEvent.change(screen.getByPlaceholderText('What needs to be done?'), {
      target: { value: 'Ship it' },
    });
    fireEvent.click(screen.getByRole('checkbox'));
    fireEvent.click(screen.getByRole('button', { name: 'Create task' }));

    // Single atomic add carries the assignment — no separate edit (no race).
    await waitFor(() => expect(mockAdd).toHaveBeenCalledTimes(1));
    expect(mockAdd).toHaveBeenCalledWith(
      expect.objectContaining({
        threadId: USER_TASKS_THREAD_ID,
        content: 'Ship it',
        assignedAgent: 'orchestrator',
        approvalMode: 'not_required',
      })
    );
    expect(mockEdit).not.toHaveBeenCalled();
  });

  it('does not assign an agent when the toggle is left off', async () => {
    mockAdd.mockResolvedValueOnce(emptyBoard(USER_TASKS_THREAD_ID));
    render(<UserTaskComposer onCreated={vi.fn()} onClose={vi.fn()} />);

    fireEvent.change(screen.getByPlaceholderText('What needs to be done?'), {
      target: { value: 'Buy milk' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Create task' }));

    await waitFor(() => expect(mockAdd).toHaveBeenCalledTimes(1));
    expect(mockEdit).not.toHaveBeenCalled();
  });

  it('disables assign-to-agent when the task is attached to a conversation', () => {
    render(<UserTaskComposer onCreated={vi.fn()} onClose={vi.fn()} />);
    fireEvent.change(screen.getByPlaceholderText('What needs to be done?'), {
      target: { value: 'Book hotel' },
    });
    const checkbox = screen.getByRole('checkbox');
    expect(checkbox).toBeEnabled();
    // Attaching to a thread takes it off the personal board — the poller doesn't
    // poll conversation threads, so auto-run is disabled there.
    fireEvent.change(screen.getByDisplayValue('Personal (no conversation)'), {
      target: { value: 't-1' },
    });
    expect(checkbox).toBeDisabled();
  });

  it('surfaces an error and keeps the modal open on failure', async () => {
    mockAdd.mockRejectedValueOnce(new Error('boom'));
    const onClose = vi.fn();
    render(<UserTaskComposer onCreated={vi.fn()} onClose={onClose} />);

    fireEvent.change(screen.getByPlaceholderText('What needs to be done?'), {
      target: { value: 'Buy milk' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Create task' }));

    await waitFor(() => expect(screen.getByText(/Couldn't create the task/)).toBeInTheDocument());
    expect(onClose).not.toHaveBeenCalled();
  });
});
