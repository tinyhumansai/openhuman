import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import ListRow from './ListRow';

describe('<ListRow />', () => {
  it('renders the label and marks itself with the list-row slot', () => {
    render(
      <ul>
        <ListRow label="allow.example.com" removeLabel="Remove" data-testid="row" />
      </ul>
    );

    const row = screen.getByTestId('row');
    expect(row).toHaveAttribute('data-slot', 'list-row');
    expect(row).toHaveTextContent('allow.example.com');
  });

  it('omits the remove control when there is no onRemove', () => {
    render(
      <ul>
        <ListRow label="pinned" removeLabel="Remove" />
      </ul>
    );

    expect(screen.queryByRole('button')).toBeNull();
  });

  it('calls onRemove from the text remove button', () => {
    const onRemove = vi.fn();
    render(
      <ul>
        <ListRow label="entry" removeLabel="Remove" onRemove={onRemove} />
      </ul>
    );

    fireEvent.click(screen.getByRole('button', { name: 'Remove' }));
    expect(onRemove).toHaveBeenCalledTimes(1);
  });

  it('keeps removeLabel as the accessible name when a removeIcon replaces the text', () => {
    const onRemove = vi.fn();
    render(
      <ul>
        <ListRow
          label="edge"
          removeLabel="Detach edge"
          removeIcon="✕"
          removeTestId="detach"
          onRemove={onRemove}
        />
      </ul>
    );

    // The painted glyph is the icon, but the a11y tree still reads the label.
    const button = screen.getByTestId('detach');
    expect(button).toHaveAccessibleName('Detach edge');
    expect(button).toHaveAttribute('title', 'Detach edge');
    expect(button).toHaveTextContent('✕');
    expect(button).not.toHaveTextContent('Detach edge');

    fireEvent.click(button);
    expect(onRemove).toHaveBeenCalledTimes(1);
  });
});
