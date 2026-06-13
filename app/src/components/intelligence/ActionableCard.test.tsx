import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { ActionableItem } from '../../types/intelligence';
import { ActionableCard } from './ActionableCard';

function makeItem(overrides: Partial<ActionableItem> = {}): ActionableItem {
  const now = new Date();
  return {
    id: 'item-1',
    title: 'Reply to Alice',
    description: 'She asked about Q4 numbers',
    source: 'email',
    priority: 'normal',
    status: 'active',
    createdAt: new Date(now.getTime() - 10 * 60 * 1000), // 10 minutes ago
    updatedAt: now,
    actionable: true,
    ...overrides,
  };
}

describe('<ActionableCard />', () => {
  it('renders title, description and the relative time meta', () => {
    render(
      <ActionableCard
        item={makeItem()}
        onComplete={() => {}}
        onDismiss={() => {}}
        onSnooze={() => {}}
      />
    );
    expect(screen.getByText('Reply to Alice')).toBeInTheDocument();
    expect(screen.getByText('She asked about Q4 numbers')).toBeInTheDocument();
    expect(screen.getByText(/10 mins ago/i)).toBeInTheDocument();
  });

  it('invokes onComplete with the item when the check button is clicked', () => {
    const item = makeItem();
    const onComplete = vi.fn();
    render(
      <ActionableCard
        item={item}
        onComplete={onComplete}
        onDismiss={() => {}}
        onSnooze={() => {}}
      />
    );
    const completeBtn = screen.getByTitle(/actionable\.complete|complete/i);
    fireEvent.click(completeBtn);
    expect(onComplete).toHaveBeenCalledWith(item);
  });

  it('invokes onDismiss with the item when the x button is clicked', () => {
    const item = makeItem();
    const onDismiss = vi.fn();
    render(
      <ActionableCard item={item} onComplete={() => {}} onDismiss={onDismiss} onSnooze={() => {}} />
    );
    const dismissBtn = screen.getByTitle(/actionable\.dismiss|dismiss/i);
    fireEvent.click(dismissBtn);
    expect(onDismiss).toHaveBeenCalledWith(item);
  });

  it('opens the snooze dropdown on click and surfaces the duration options', () => {
    render(
      <ActionableCard
        item={makeItem()}
        onComplete={() => {}}
        onDismiss={() => {}}
        onSnooze={() => {}}
      />
    );
    fireEvent.click(screen.getByTitle(/actionable\.snooze|snooze/i));
    expect(screen.getByText('1 hour')).toBeInTheDocument();
    expect(screen.getByText('6 hours')).toBeInTheDocument();
    expect(screen.getByText('24 hours')).toBeInTheDocument();
  });

  it('invokes onSnooze with the duration after the animation timer fires', () => {
    vi.useFakeTimers();
    try {
      const item = makeItem();
      const onSnooze = vi.fn();
      render(
        <ActionableCard
          item={item}
          onComplete={() => {}}
          onDismiss={() => {}}
          onSnooze={onSnooze}
        />
      );
      fireEvent.click(screen.getByTitle(/actionable\.snooze|snooze/i));
      fireEvent.click(screen.getByText('1 hour'));
      // The 200ms animation-out delay must elapse before onSnooze fires.
      vi.advanceTimersByTime(250);
      expect(onSnooze).toHaveBeenCalledWith(item, 60 * 60 * 1000);
    } finally {
      vi.useRealTimers();
    }
  });

  it('renders the "new" badge for items younger than 5 minutes', () => {
    render(
      <ActionableCard
        item={makeItem({ createdAt: new Date(Date.now() - 60_000) })}
        onComplete={() => {}}
        onDismiss={() => {}}
        onSnooze={() => {}}
      />
    );
    // i18n key falls back to "actionable.new" when no provider is mounted.
    expect(screen.getByText(/actionable\.new|new/i)).toBeInTheDocument();
  });

  it('honors a custom sourceLabel when provided', () => {
    render(
      <ActionableCard
        item={makeItem({ sourceLabel: 'Gmail' })}
        onComplete={() => {}}
        onDismiss={() => {}}
        onSnooze={() => {}}
      />
    );
    expect(screen.getByText('Gmail')).toBeInTheDocument();
  });
});
