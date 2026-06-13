import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { TaskBoard } from '../../../../types/turnState';
import { TaskKanbanBoard } from '../TaskKanbanBoard';

const board: TaskBoard = {
  threadId: 'thread-1',
  updatedAt: '2026-05-04T10:00:05Z',
  cards: [
    {
      id: 'task-1',
      title: 'Draft plan',
      status: 'todo',
      objective: 'Prepare the implementation handoff',
      plan: ['Read existing board code', 'Update shared card shape'],
      assignedAgent: 'planner',
      allowedTools: ['todo', 'spawn_subagent'],
      approvalMode: 'required',
      acceptanceCriteria: ['Schema round-trips'],
      evidence: ['unit tests'],
      notes: 'Scope frontend and backend work',
      order: 0,
      updatedAt: '2026-05-04T10:00:05Z',
    },
    {
      id: 'task-2',
      title: 'Wait for token',
      status: 'blocked',
      blocker: 'Missing credentials',
      order: 1,
      updatedAt: '2026-05-04T10:00:05Z',
    },
  ],
};

describe('TaskKanbanBoard', () => {
  it('renders five columns including Blocked as its own column', () => {
    render(<TaskKanbanBoard board={board} />);

    // The board now surfaces five columns; Blocked is its own column.
    expect(screen.getByText('Pending')).toBeInTheDocument();
    expect(screen.getByText('Working')).toBeInTheDocument();
    expect(screen.getByText('Done')).toBeInTheDocument();
    // Blocked is now a visible column (not bucketed into Done)
    expect(screen.getByText('Blocked')).toBeInTheDocument();
    expect(screen.queryByText('To do')).not.toBeInTheDocument();
    expect(screen.getByText('Draft plan')).toBeInTheDocument();
    // The blocked card is rendered in Blocked column with its blocker reason.
    expect(screen.getByText('Wait for token')).toBeInTheDocument();
    expect(screen.getByText('Prepare the implementation handoff')).toBeInTheDocument();
    expect(screen.getByText('planner')).toBeInTheDocument();
    expect(screen.getByText('approval')).toBeInTheDocument();
    expect(screen.getByText('Scope frontend and backend work')).toBeInTheDocument();
    expect(screen.getByText('Missing credentials')).toBeInTheDocument();
  });

  it('opens a task brief with plan, tools, criteria, and evidence', () => {
    render(<TaskKanbanBoard board={board} />);

    fireEvent.click(screen.getAllByText('Task brief')[0]);

    expect(screen.getByRole('heading', { name: 'Draft plan' })).toBeInTheDocument();
    expect(screen.getByText('Required before execution')).toBeInTheDocument();
    expect(screen.getByText('Read existing board code')).toBeInTheDocument();
    expect(screen.getByText('spawn_subagent')).toBeInTheDocument();
    expect(screen.getByText('Schema round-trips')).toBeInTheDocument();
    expect(screen.getByText('unit tests')).toBeInTheDocument();
  });

  it('calls onMove with awaiting_approval when a todo card is moved right', () => {
    const onMove = vi.fn();
    render(<TaskKanbanBoard board={board} onMove={onMove} />);

    const moveRightButtons = screen.getAllByLabelText('Move right');
    fireEvent.click(moveRightButtons[0]);

    // todo → awaiting_approval (new second column)
    expect(onMove).toHaveBeenCalledWith(board.cards[0], 'awaiting_approval');
  });

  it('lets users edit a task brief and save the updated card', () => {
    const onUpdateCard = vi.fn();
    render(<TaskKanbanBoard board={board} onUpdateCard={onUpdateCard} />);

    fireEvent.click(screen.getAllByText('Task brief')[0]);
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Updated plan' } });
    fireEvent.change(screen.getByLabelText('Assigned agent'), {
      target: { value: 'code_executor' },
    });
    fireEvent.change(screen.getByLabelText('Plan'), {
      target: { value: 'Inspect files\nPatch UI' },
    });
    fireEvent.change(screen.getByLabelText('Allowed tools'), {
      target: { value: 'todo\nfile_read' },
    });
    fireEvent.change(screen.getByLabelText('Approval'), { target: { value: 'not_required' } });
    fireEvent.change(screen.getByLabelText('Status'), { target: { value: 'in_progress' } });
    fireEvent.click(screen.getByText('Save changes'));

    expect(onUpdateCard).toHaveBeenCalledWith(
      board.cards[0],
      expect.objectContaining({
        title: 'Updated plan',
        assignedAgent: 'code_executor',
        plan: ['Inspect files', 'Patch UI'],
        allowedTools: ['todo', 'file_read'],
        approvalMode: 'not_required',
        status: 'in_progress',
      })
    );
  });

  it('shows not-required approval details and danger tone blockers', () => {
    render(
      <TaskKanbanBoard
        board={{
          ...board,
          cards: [
            {
              ...board.cards[0],
              approvalMode: 'not_required',
              blocker: 'External dependency is down',
            },
          ],
        }}
      />
    );

    fireEvent.click(screen.getByText('Task brief'));

    expect(screen.getByText('Not required')).toBeInTheDocument();
    expect(screen.getByText('External dependency is down')).toHaveClass('text-coral-600');
  });

  it('buckets ready→in_progress column and rejected→done column', () => {
    render(
      <TaskKanbanBoard
        board={{
          ...board,
          cards: [
            { id: 'r', title: 'Ready card', status: 'ready', order: 0, updatedAt: '' },
            { id: 'x', title: 'Rejected card', status: 'rejected', order: 1, updatedAt: '' },
          ],
        }}
      />
    );

    expect(screen.getByText('Ready card')).toBeInTheDocument();
    expect(screen.getByText('Rejected card')).toBeInTheDocument();
    // ready card gets a "Ready to start" badge
    expect(screen.getByText('Ready to start')).toBeInTheDocument();
    // rejected card gets a "Rejected" badge
    expect(screen.getByText('Rejected')).toBeInTheDocument();
  });

  it('renders awaiting_approval card in its own Awaiting Approval column', () => {
    render(
      <TaskKanbanBoard
        board={{
          ...board,
          cards: [
            {
              id: 'ap',
              title: 'Awaiting approval task',
              status: 'awaiting_approval',
              order: 0,
              updatedAt: '',
            },
          ],
        }}
      />
    );

    expect(screen.getByText('Awaiting approval task')).toBeInTheDocument();
    // The column header should be rendered
    expect(screen.getByText('Awaiting Approval')).toBeInTheDocument();
  });

  it('shows "Needs your input" banner in Blocked column when it has cards', () => {
    render(<TaskKanbanBoard board={board} />);

    // board has a blocked card ("Wait for token") — banner should show
    expect(screen.getByText('These tasks need your input')).toBeInTheDocument();
  });

  it('renders empty column placeholder text when a column has no cards', () => {
    render(
      <TaskKanbanBoard
        board={{
          threadId: 'thread-empty',
          updatedAt: '',
          cards: [{ id: 't1', title: 'Only pending', status: 'todo', order: 0, updatedAt: '' }],
        }}
      />
    );

    // Done column should show "No tasks" placeholder
    const emptyPlaceholders = screen.getAllByText('No tasks');
    expect(emptyPlaceholders.length).toBeGreaterThan(0);
  });

  it('shows evidence badge on card when evidence array is non-empty', () => {
    render(
      <TaskKanbanBoard
        board={{
          ...board,
          cards: [
            {
              id: 'ev',
              title: 'Evidence task',
              status: 'todo',
              order: 0,
              updatedAt: '',
              evidence: ['test result A', 'test result B'],
            },
          ],
        }}
      />
    );

    // Evidence badge shows count
    expect(screen.getByText('Evidence (2)')).toBeInTheDocument();
  });

  it('empty board still renders all five columns', () => {
    render(<TaskKanbanBoard board={{ threadId: 'empty-board', updatedAt: '', cards: [] }} />);

    expect(screen.getByText('Pending')).toBeInTheDocument();
    expect(screen.getByText('Awaiting Approval')).toBeInTheDocument();
    expect(screen.getByText('Working')).toBeInTheDocument();
    expect(screen.getByText('Blocked')).toBeInTheDocument();
    expect(screen.getByText('Done')).toBeInTheDocument();
  });

  it('drag-and-drop: dragStart on card then drop on a column calls onMove', () => {
    const onMove = vi.fn();
    render(
      <TaskKanbanBoard
        board={{
          threadId: 'dnd-board',
          updatedAt: '',
          cards: [
            { id: 'drag-card', title: 'Draggable task', status: 'todo', order: 0, updatedAt: '' },
          ],
        }}
        onMove={onMove}
      />
    );

    const card = screen.getByText('Draggable task').closest('article')!;
    // Simulate drag start
    fireEvent.dragStart(card, { dataTransfer: { setData: vi.fn(), effectAllowed: 'move' } });

    // Find the Done column section and simulate drop
    const sections = document.querySelectorAll('section');
    // sections[0]=Pending, [1]=AwaitingApproval, [2]=InProgress, [3]=Blocked, [4]=Done
    const doneSection = sections[4];
    fireEvent.dragOver(doneSection, { dataTransfer: { dropEffect: 'move' } });
    fireEvent.drop(doneSection, {
      dataTransfer: {
        getData: (key: string) => (key === 'application/x-task-card-id' ? 'drag-card' : ''),
      },
    });

    expect(onMove).toHaveBeenCalledWith(expect.objectContaining({ id: 'drag-card' }), 'done');
  });

  it('drag-and-drop: dropping a card on its own column does not call onMove', () => {
    const onMove = vi.fn();
    render(
      <TaskKanbanBoard
        board={{
          threadId: 'dnd-noop-board',
          updatedAt: '',
          cards: [
            { id: 'same-col-card', title: 'Pending task', status: 'todo', order: 0, updatedAt: '' },
          ],
        }}
        onMove={onMove}
      />
    );

    const card = screen.getByText('Pending task').closest('article')!;
    fireEvent.dragStart(card, { dataTransfer: { setData: vi.fn(), effectAllowed: 'move' } });

    // sections[0] = Pending — the todo card's own column
    const pendingSection = document.querySelectorAll('section')[0];
    fireEvent.dragOver(pendingSection, { dataTransfer: { dropEffect: 'move' } });
    fireEvent.drop(pendingSection, {
      dataTransfer: {
        getData: (key: string) => (key === 'application/x-task-card-id' ? 'same-col-card' : ''),
      },
    });

    expect(onMove).not.toHaveBeenCalled();
  });

  it('drag-and-drop: a disabled board does not call onMove on drop', () => {
    const onMove = vi.fn();
    render(
      <TaskKanbanBoard
        board={{
          threadId: 'dnd-disabled-board',
          updatedAt: '',
          cards: [
            { id: 'disabled-card', title: 'Locked task', status: 'todo', order: 0, updatedAt: '' },
          ],
        }}
        onMove={onMove}
        disabled
      />
    );

    const card = screen.getByText('Locked task').closest('article')!;
    // Disabled cards must not be draggable from the source side …
    expect(card.getAttribute('draggable')).toBe('false');

    // … and a drop event must be a no-op even if one is dispatched.
    const doneSection = document.querySelectorAll('section')[4];
    fireEvent.drop(doneSection, {
      dataTransfer: {
        getData: (key: string) => (key === 'application/x-task-card-id' ? 'disabled-card' : ''),
      },
    });

    expect(onMove).not.toHaveBeenCalled();
  });

  it('arrow buttons still call onMove as a11y fallback', () => {
    const onMove = vi.fn();
    render(
      <TaskKanbanBoard
        board={{
          threadId: 'a11y-board',
          updatedAt: '',
          cards: [{ id: 'a1', title: 'Arrow task', status: 'todo', order: 0, updatedAt: '' }],
        }}
        onMove={onMove}
      />
    );

    const moveRightButtons = screen.getAllByLabelText('Move right');
    fireEvent.click(moveRightButtons[0]);

    expect(onMove).toHaveBeenCalledWith(expect.objectContaining({ id: 'a1' }), 'awaiting_approval');
  });
});
