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
  todosApi: { add: vi.fn() },
}));

const mockAdd = vi.mocked(todosApi.add);

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
