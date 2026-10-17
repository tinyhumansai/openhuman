import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { type TodoItem, todoProgress, TodoProgressCard, TodoReceipt } from './todo-list';

const items: TodoItem[] = [
  { id: '1', text: 'Read the spec', status: 'done' },
  { id: '2', text: 'Write the code', status: 'active' },
  { id: '3', text: 'Ship it', status: 'pending' },
];

describe('todoProgress', () => {
  it('names the first active step and its position', () => {
    const progress = todoProgress(items);
    expect(progress).toMatchObject({ done: 1, total: 3, position: 2, anyActive: true });
    expect(progress.current?.text).toBe('Write the code');
  });

  it('falls back to the first pending step, then the last done one', () => {
    const pending = todoProgress([
      { id: '1', text: 'A', status: 'done' },
      { id: '2', text: 'B', status: 'pending' },
    ]);
    expect(pending.current?.text).toBe('B');
    const finished = todoProgress([
      { id: '1', text: 'A', status: 'done' },
      { id: '2', text: 'B', status: 'done' },
    ]);
    expect(finished).toMatchObject({ allDone: true, position: 2 });
    expect(finished.current?.text).toBe('B');
  });

  it('counts a skipped step as closed, never as the current one', () => {
    const progress = todoProgress([
      { id: '1', text: 'A', status: 'done' },
      { id: '2', text: 'B', status: 'skipped', reason: 'not needed' },
    ]);
    expect(progress).toMatchObject({ done: 2, total: 2, allDone: true });
    expect(progress.current?.text).toBe('A');
  });

  it('handles an empty list', () => {
    expect(todoProgress([])).toMatchObject({ total: 0, position: 0, allDone: false });
  });
});

describe('TodoProgressCard', () => {
  const labels = { title: 'Todos', completedLabel: 'Completed', countLabel: '1 of 3' };

  it('uses a compact floating surface across the composer width', () => {
    const { container } = render(
      <TodoProgressCard items={items} open onOpenChange={vi.fn()} {...labels} />
    );
    const card = container.querySelector('[data-slot="todo-progress-card"]');
    expect(card).toHaveClass('max-w-none', 'bg-background');
    expect(card).not.toHaveClass('bg-transparent', 'backdrop-blur-md');
    expect(screen.getAllByTestId('todo-item')[0]).toHaveClass('text-xs', 'py-0');
  });

  it('collapsed shows the current step and pos/total, not the list', () => {
    render(<TodoProgressCard items={items} open={false} onOpenChange={vi.fn()} {...labels} />);
    const toggle = screen.getByRole('button', { name: 'Todos' });
    expect(toggle).toHaveAttribute('aria-expanded', 'false');
    expect(toggle).toHaveTextContent('Write the code');
    expect(screen.getByTestId('todo-progress-count')).toHaveTextContent('2/3');
    // The steps stay mounted but hidden, so they take no space and are not visible.
    expect(screen.getByText('Ship it')).not.toBeVisible();
  });

  it('expanded shows the count label and every step', () => {
    const onOpenChange = vi.fn();
    render(<TodoProgressCard items={items} open onOpenChange={onOpenChange} {...labels} />);
    expect(screen.getByTestId('todo-progress-count')).toHaveTextContent('1 of 3');
    expect(screen.getByText('Ship it')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Todos' }));
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it('says completed once every step is done', () => {
    render(
      <TodoProgressCard
        items={items.map(item => ({ ...item, status: 'done' as const }))}
        open={false}
        onOpenChange={vi.fn()}
        {...labels}
      />
    );
    expect(screen.getByTestId('todo-progress-count')).toHaveTextContent('Completed');
  });
});

describe('TodoReceipt', () => {
  it('is one line that expands to the snapshot', () => {
    const onOpenChange = vi.fn();
    const { rerender } = render(
      <TodoReceipt items={items} label="Progress 1/3" open={false} onOpenChange={onOpenChange} />
    );
    expect(screen.getByRole('button')).toHaveTextContent('Progress 1/3 · Write the code');
    expect(screen.queryByText('Ship it')).toBeNull();
    fireEvent.click(screen.getByRole('button'));
    expect(onOpenChange).toHaveBeenCalledWith(true);
    rerender(<TodoReceipt items={items} label="Progress 1/3" open onOpenChange={onOpenChange} />);
    expect(screen.getByText('Ship it')).toBeInTheDocument();
  });
});
