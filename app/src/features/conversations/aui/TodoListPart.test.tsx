import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { mapCoreTodoStatus, toAuiTodoItems, TodoListPart } from './TodoListPart';

describe('mapCoreTodoStatus', () => {
  it('maps in_progress to active', () => {
    expect(mapCoreTodoStatus('in_progress')).toBe('active');
  });
  it('maps completed to done', () => {
    expect(mapCoreTodoStatus('completed')).toBe('done');
  });
  it('maps pending to pending', () => {
    expect(mapCoreTodoStatus('pending')).toBe('pending');
  });
  it('maps cancelled to skipped, never pending', () => {
    expect(mapCoreTodoStatus('cancelled')).toBe('skipped');
  });
  it('maps an unknown status to pending', () => {
    expect(mapCoreTodoStatus('bogus')).toBe('pending');
    expect(mapCoreTodoStatus(undefined)).toBe('pending');
  });
});

describe('toAuiTodoItems', () => {
  it('maps core wire items to TodoItem, using content+index as a stable id', () => {
    const items = toAuiTodoItems([
      { content: 'Write tests', status: 'pending' },
      { content: 'Ship it', status: 'completed' },
    ]);
    expect(items).toEqual([
      { id: '0-Write tests', text: 'Write tests', status: 'pending' },
      { id: '1-Ship it', text: 'Ship it', status: 'done' },
    ]);
  });

  it('carries a cancelled step as skipped with its reason', () => {
    expect(
      toAuiTodoItems([{ content: 'Deploy', status: 'cancelled', reason: 'not asked for' }])
    ).toEqual([{ id: '0-Deploy', text: 'Deploy', status: 'skipped', reason: 'not asked for' }]);
  });

  it('drops items with no non-empty content', () => {
    expect(toAuiTodoItems([{ content: '', status: 'pending' }, { status: 'pending' }])).toEqual([]);
  });

  it('returns an empty array for a non-array payload', () => {
    expect(toAuiTodoItems(undefined)).toEqual([]);
    expect(toAuiTodoItems({})).toEqual([]);
  });
});

const baseProps = {
  type: 'tool-call' as const,
  toolName: 'todo',
  toolCallId: 'call-1',
  argsText: '{}',
  addResult: () => {},
  resume: () => {},
  respondToApproval: async () => {},
};

describe('TodoListPart', () => {
  it('renders a one-line receipt that expands to the snapshot', () => {
    render(
      <TodoListPart
        {...baseProps}
        toolCallId="call-receipt"
        args={{} as never}
        result={
          {
            todos: [
              { content: 'Write tests', status: 'completed' },
              { content: 'Ship it', status: 'in_progress' },
              { content: 'Celebrate', status: 'pending' },
            ],
          } as never
        }
        status={{ type: 'complete' }}
      />
    );
    const toggle = screen.getByRole('button', { expanded: false });
    expect(toggle).toHaveTextContent('1/3');
    expect(toggle).toHaveTextContent('Ship it');
    // Collapsed: the full list is not repeated in the transcript.
    expect(screen.queryByText('Celebrate')).toBeNull();

    fireEvent.click(toggle);
    expect(screen.getByRole('button', { expanded: true })).toBeInTheDocument();
    expect(screen.getByText('Celebrate')).toBeInTheDocument();
  });

  it('renders nothing when there are no items', () => {
    const { container } = render(
      <TodoListPart
        {...baseProps}
        args={{ todos: [] } as never}
        result={undefined}
        status={{ type: 'running' }}
      />
    );
    expect(container).toBeEmptyDOMElement();
  });
});
