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
  it('renders kanban columns, cards, notes, and blockers', () => {
    render(<TaskKanbanBoard board={board} />);

    expect(screen.getByText('To do')).toBeInTheDocument();
    expect(screen.getByText('In progress')).toBeInTheDocument();
    expect(screen.getByText('Blocked')).toBeInTheDocument();
    expect(screen.getByText('Done')).toBeInTheDocument();
    expect(screen.getByText('Draft plan')).toBeInTheDocument();
    expect(screen.getByText('Scope frontend and backend work')).toBeInTheDocument();
    expect(screen.getByText('Missing credentials')).toBeInTheDocument();
  });

  it('calls onMove with the next status when a card is moved', () => {
    const onMove = vi.fn();
    render(<TaskKanbanBoard board={board} onMove={onMove} />);

    const moveRightButtons = screen.getAllByLabelText('Move right');
    fireEvent.click(moveRightButtons[0]);

    expect(onMove).toHaveBeenCalledWith(board.cards[0], 'in_progress');
  });
});
