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
  it('renders the three columns, cards, notes, and blockers', () => {
    render(<TaskKanbanBoard board={board} />);

    // The board surfaces exactly three columns; `blocked` is bucketed into Done.
    expect(screen.getByText('Pending')).toBeInTheDocument();
    expect(screen.getByText('Working')).toBeInTheDocument();
    expect(screen.getByText('Done')).toBeInTheDocument();
    expect(screen.queryByText('To do')).not.toBeInTheDocument();
    expect(screen.queryByText('Blocked')).not.toBeInTheDocument();
    expect(screen.getByText('Draft plan')).toBeInTheDocument();
    // The blocked card is still rendered (under Done) with its blocker reason.
    expect(screen.getByText('Wait for token')).toBeInTheDocument();
    expect(screen.getByText('Prepare the implementation handoff')).toBeInTheDocument();
    expect(screen.getByText('planner')).toBeInTheDocument();
    expect(screen.getByText('approval')).toBeInTheDocument();
    expect(screen.getByText('Scope frontend and backend work')).toBeInTheDocument();
    expect(screen.getByText('Missing credentials')).toBeInTheDocument();
  });

  it('opens a task brief with plan, tools, criteria, and evidence', () => {
    render(<TaskKanbanBoard board={board} />);

    fireEvent.click(screen.getByText('Task brief'));

    expect(screen.getByRole('heading', { name: 'Draft plan' })).toBeInTheDocument();
    expect(screen.getByText('Required before execution')).toBeInTheDocument();
    expect(screen.getByText('Read existing board code')).toBeInTheDocument();
    expect(screen.getByText('spawn_subagent')).toBeInTheDocument();
    expect(screen.getByText('Schema round-trips')).toBeInTheDocument();
    expect(screen.getByText('unit tests')).toBeInTheDocument();
  });

  it('calls onMove with the next status when a card is moved', () => {
    const onMove = vi.fn();
    render(<TaskKanbanBoard board={board} onMove={onMove} />);

    const moveRightButtons = screen.getAllByLabelText('Move right');
    fireEvent.click(moveRightButtons[0]);

    expect(onMove).toHaveBeenCalledWith(board.cards[0], 'in_progress');
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
});
