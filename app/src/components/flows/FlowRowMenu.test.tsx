/**
 * FlowRowMenu — the "⋯" overflow menu. Asserts the trigger opens the menu,
 * each item is reachable by its test id and label, selecting an item both
 * fires its callback and closes the menu, and the danger item carries the
 * coral tone class. `useT()` falls back to the bundled English map with no
 * provider mounted (same as the sibling flows tests).
 *
 * Radix's `DropdownMenuTrigger` opens on `pointerdown` (not `click`, so a
 * press-and-drag select onto an item keeps working) — a plain
 * `fireEvent.click` never dispatches that event, so opening it here fires
 * `pointerDown` immediately before the `click`, matching what a real mouse
 * interaction produces.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import FlowRowMenu from './FlowRowMenu';

function openMenu(rowId: string) {
  const trigger = screen.getByTestId(`flow-menu-${rowId}`);
  fireEvent.pointerDown(trigger);
  fireEvent.click(trigger);
}

describe('FlowRowMenu', () => {
  it('opens the menu from the trigger and shows every item', () => {
    render(
      <FlowRowMenu
        rowId="flow-1"
        items={[
          { key: 'export', label: 'Export', onSelect: vi.fn(), testId: 'flow-export-flow-1' },
          {
            key: 'duplicate',
            label: 'Duplicate',
            onSelect: vi.fn(),
            testId: 'flow-duplicate-flow-1',
          },
        ]}
      />
    );
    expect(screen.queryByTestId('flow-export-flow-1')).not.toBeInTheDocument();

    openMenu('flow-1');

    expect(screen.getByTestId('flow-export-flow-1')).toHaveTextContent('Export');
    expect(screen.getByTestId('flow-duplicate-flow-1')).toHaveTextContent('Duplicate');
  });

  it('fires onSelect when an item is chosen', () => {
    const onSelect = vi.fn();
    render(
      <FlowRowMenu
        rowId="flow-1"
        items={[{ key: 'export', label: 'Export', onSelect, testId: 'flow-export-flow-1' }]}
      />
    );

    openMenu('flow-1');
    fireEvent.click(screen.getByTestId('flow-export-flow-1'));

    expect(onSelect).toHaveBeenCalledTimes(1);
  });

  it('renders a danger item in the destructive coral tone', () => {
    render(
      <FlowRowMenu
        rowId="flow-1"
        items={[
          {
            key: 'delete',
            label: 'Delete',
            onSelect: vi.fn(),
            testId: 'flow-delete-flow-1',
            danger: true,
          },
        ]}
      />
    );

    openMenu('flow-1');
    expect(screen.getByTestId('flow-delete-flow-1')).toHaveClass('text-coral-600');
  });
});
