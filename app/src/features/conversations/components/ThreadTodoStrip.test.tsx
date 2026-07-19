import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { TaskBoard, TaskBoardCard } from '../../../types/turnState';
import { ThreadTodoStrip } from './ThreadTodoStrip';

// Echo i18n keys so we can assert on the stable key string.
vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

function card(partial: Partial<TaskBoardCard>): TaskBoardCard {
  return {
    id: 'c1',
    title: 'Do thing',
    status: 'todo',
    order: 0,
    updatedAt: '',
    ...partial,
  } as TaskBoardCard;
}

function board(cards: TaskBoardCard[]): TaskBoard {
  return { threadId: 't1', cards, updatedAt: '' };
}

describe('ThreadTodoStrip', () => {
  it('renders nothing when board is null', () => {
    const { container } = render(<ThreadTodoStrip board={null} />);
    expect(container).toBeEmptyDOMElement();
  });

  it('renders nothing when there are no active cards', () => {
    const { container } = render(
      <ThreadTodoStrip
        board={board([card({ id: 'a', status: 'done' }), card({ id: 'b', status: 'rejected' })])}
      />
    );
    expect(container).toBeEmptyDOMElement();
  });

  it('lists active cards and hides done/rejected ones', () => {
    render(
      <ThreadTodoStrip
        board={board([
          card({ id: 'a', title: 'Active work', status: 'in_progress' }),
          card({ id: 'b', title: 'Queued work', status: 'todo' }),
          card({ id: 'c', title: 'Finished work', status: 'done' }),
          card({ id: 'd', title: 'Dropped work', status: 'rejected' }),
        ])}
      />
    );
    expect(screen.getByText('Active work')).toBeInTheDocument();
    expect(screen.getByText('Queued work')).toBeInTheDocument();
    expect(screen.queryByText('Finished work')).not.toBeInTheDocument();
    expect(screen.queryByText('Dropped work')).not.toBeInTheDocument();
  });

  it('shows a done/total progress count excluding rejected cards', () => {
    render(
      <ThreadTodoStrip
        board={board([
          card({ id: 'a', status: 'in_progress' }),
          card({ id: 'b', status: 'done' }),
          card({ id: 'c', status: 'rejected' }),
        ])}
      />
    );
    // 1 done out of 2 tracked (rejected excluded entirely).
    expect(screen.getByText('1/2')).toBeInTheDocument();
  });

  it('orders active cards by their `order` field', () => {
    render(
      <ThreadTodoStrip
        board={board([
          card({ id: 'a', title: 'Second', status: 'todo', order: 5 }),
          card({ id: 'b', title: 'First', status: 'todo', order: 1 }),
        ])}
      />
    );
    const items = screen.getAllByRole('listitem').map(li => li.textContent);
    expect(items[0]).toContain('First');
    expect(items[1]).toContain('Second');
  });

  it('falls back to objective then id when title is blank', () => {
    render(
      <ThreadTodoStrip
        board={board([
          card({ id: 'a', title: '   ', objective: 'Ship it', status: 'todo' }),
          card({ id: 'only-id', title: '', objective: null, status: 'todo' }),
        ])}
      />
    );
    expect(screen.getByText('Ship it')).toBeInTheDocument();
    expect(screen.getByText('only-id')).toBeInTheDocument();
  });

  it('collapses and expands the list when the header is clicked', () => {
    render(
      <ThreadTodoStrip board={board([card({ id: 'a', title: 'Active work', status: 'todo' })])} />
    );
    expect(screen.getByText('Active work')).toBeInTheDocument();
    const header = screen.getByRole('button', { expanded: true });
    fireEvent.click(header);
    expect(screen.queryByText('Active work')).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { expanded: false }));
    expect(screen.getByText('Active work')).toBeInTheDocument();
  });

  it('renders Approve/Reject only for awaiting_approval cards when onDecidePlan is provided', () => {
    const onDecidePlan = vi.fn();
    render(
      <ThreadTodoStrip
        board={board([
          card({ id: 'parked', title: 'Needs sign-off', status: 'awaiting_approval' }),
          card({ id: 'plain', title: 'Just working', status: 'in_progress' }),
        ])}
        onDecidePlan={onDecidePlan}
      />
    );
    // Exactly one Approve and one Reject — only the parked card has them.
    expect(screen.getAllByText('chat.approval.approve')).toHaveLength(1);
    expect(screen.getAllByText('chat.approval.deny')).toHaveLength(1);

    fireEvent.click(screen.getByText('chat.approval.approve'));
    expect(onDecidePlan).toHaveBeenCalledWith(expect.objectContaining({ id: 'parked' }), true);
    fireEvent.click(screen.getByText('chat.approval.deny'));
    expect(onDecidePlan).toHaveBeenCalledWith(expect.objectContaining({ id: 'parked' }), false);
  });

  it('surfaces the blocker reason for blocked cards', () => {
    render(
      <ThreadTodoStrip
        board={board([
          card({ id: 'b', title: 'Stuck step', status: 'blocked', blocker: 'needs API key' }),
        ])}
      />
    );
    expect(screen.getByText('Stuck step')).toBeInTheDocument();
    expect(screen.getByText('needs API key')).toBeInTheDocument();
  });

  it('renders a View work jump only for cards with a session when onViewSession is provided', () => {
    const onViewSession = vi.fn();
    render(
      <ThreadTodoStrip
        board={board([
          card({
            id: 'linked',
            title: 'Has session',
            status: 'in_progress',
            sessionThreadId: 's1',
          }),
          card({ id: 'plain', title: 'No session', status: 'in_progress' }),
        ])}
        onViewSession={onViewSession}
      />
    );
    expect(screen.getAllByText('conversations.taskKanban.viewWork')).toHaveLength(1);
    fireEvent.click(screen.getByText('conversations.taskKanban.viewWork'));
    expect(onViewSession).toHaveBeenCalledWith(expect.objectContaining({ id: 'linked' }));
  });

  it('stays fully read-only (no approve/reject) when onDecidePlan is omitted', () => {
    render(
      <ThreadTodoStrip
        board={board([
          card({ id: 'parked', title: 'Needs sign-off', status: 'awaiting_approval' }),
        ])}
      />
    );
    expect(screen.getByText('Needs sign-off')).toBeInTheDocument();
    expect(screen.queryByText('chat.approval.approve')).not.toBeInTheDocument();
    expect(screen.queryByText('chat.approval.deny')).not.toBeInTheDocument();
  });
});
