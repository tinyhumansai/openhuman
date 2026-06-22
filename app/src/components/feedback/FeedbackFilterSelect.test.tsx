import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import FeedbackFilterSelect from './FeedbackFilterSelect';

const OPTIONS = [
  { value: 'all', label: 'All types' },
  { value: 'feature', label: 'Feature' },
  { value: 'bug', label: 'Bug' },
];

/** Resolve the option the listbox currently points at via aria-activedescendant. */
function activeOptionText(listbox: HTMLElement): string {
  const id = listbox.getAttribute('aria-activedescendant');
  return (id && document.getElementById(id)?.textContent) || '';
}

describe('<FeedbackFilterSelect />', () => {
  it('shows the current selection on the trigger', () => {
    render(
      <FeedbackFilterSelect
        value="feature"
        options={OPTIONS}
        onChange={() => {}}
        ariaLabel="Type"
      />
    );
    expect(screen.getByRole('button', { name: 'Type' })).toHaveTextContent('Feature');
  });

  it('opens the menu and selects an option', () => {
    const onChange = vi.fn();
    render(
      <FeedbackFilterSelect value="all" options={OPTIONS} onChange={onChange} ariaLabel="Type" />
    );

    expect(screen.queryByRole('listbox')).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Type' }));
    expect(screen.getByRole('listbox')).toBeInTheDocument();

    expect(screen.getByRole('option', { name: 'Bug' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Bug' }));
    expect(onChange).toHaveBeenCalledWith('bug');
    // Menu closes after selection.
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument();
  });

  it('closes on Escape', () => {
    render(
      <FeedbackFilterSelect value="all" options={OPTIONS} onChange={() => {}} ariaLabel="Type" />
    );
    fireEvent.click(screen.getByRole('button', { name: 'Type' }));
    expect(screen.getByRole('listbox')).toBeInTheDocument();
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument();
  });

  it('opens with arrow keys, focuses the popover, and marks the selected option active', () => {
    render(
      <FeedbackFilterSelect
        value="feature"
        options={OPTIONS}
        onChange={() => {}}
        ariaLabel="Type"
      />
    );
    const trigger = screen.getByRole('button', { name: 'Type' });
    trigger.focus();
    fireEvent.keyDown(trigger, { key: 'ArrowDown' });

    const listbox = screen.getByRole('listbox');
    // The popover takes focus, and the active descendant is the current selection.
    expect(listbox).toHaveFocus();
    expect(activeOptionText(listbox)).toContain('Feature');
  });

  it('moves the active option with Up/Down/Home/End and selects with Enter', () => {
    const onChange = vi.fn();
    render(
      <FeedbackFilterSelect value="all" options={OPTIONS} onChange={onChange} ariaLabel="Type" />
    );
    fireEvent.click(screen.getByRole('button', { name: 'Type' }));
    const listbox = screen.getByRole('listbox');

    expect(activeOptionText(listbox)).toContain('All types');
    fireEvent.keyDown(listbox, { key: 'ArrowDown' }); // all -> Feature
    expect(activeOptionText(listbox)).toContain('Feature');
    fireEvent.keyDown(listbox, { key: 'Home' }); // -> All types
    expect(activeOptionText(listbox)).toContain('All types');
    fireEvent.keyDown(listbox, { key: 'ArrowUp' }); // wraps -> Bug
    expect(activeOptionText(listbox)).toContain('Bug');
    fireEvent.keyDown(listbox, { key: 'End' }); // -> Bug
    expect(activeOptionText(listbox)).toContain('Bug');
    fireEvent.keyDown(listbox, { key: 'ArrowDown' }); // wraps -> All types
    expect(activeOptionText(listbox)).toContain('All types');

    fireEvent.keyDown(listbox, { key: 'Enter' });
    expect(onChange).toHaveBeenCalledWith('all');
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument();
  });
});
