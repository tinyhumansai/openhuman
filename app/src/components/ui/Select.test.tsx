import { fireEvent, render, screen } from '@testing-library/react';
import { createRef } from 'react';
import { describe, expect, it, vi } from 'vitest';

import { SelectContent, SelectItem, SelectRoot, SelectTrigger, SelectValue } from './Select';

const RAW_PALETTE = /\b(?:bg|text|border|ring)-(?:neutral|stone|slate|canvas|white|black)\b/;

interface HarnessProps {
  onValueChange?: (next: string) => void;
  defaultValue?: string;
  triggerProps?: Record<string, unknown>;
}

const Harness = ({ onValueChange, defaultValue, triggerProps }: HarnessProps) => (
  <SelectRoot defaultValue={defaultValue} onValueChange={onValueChange}>
    <SelectTrigger aria-label="Tone" data-testid="tone-trigger" {...triggerProps}>
      <SelectValue placeholder="Pick a tone" />
    </SelectTrigger>
    <SelectContent>
      <SelectItem value="calm">Calm</SelectItem>
      <SelectItem value="direct">Direct</SelectItem>
      <SelectItem value="playful">Playful</SelectItem>
    </SelectContent>
  </SelectRoot>
);

describe('Select', () => {
  it('renders a combobox trigger showing the placeholder when unset', () => {
    render(<Harness />);

    const trigger = screen.getByTestId('tone-trigger');
    expect(trigger).toHaveAttribute('data-slot', 'select-trigger');
    expect(trigger).toHaveAttribute('data-placeholder');
    expect(trigger).toHaveTextContent('Pick a tone');
  });

  it('shows the selected item text on the trigger', () => {
    render(<Harness defaultValue="direct" />);
    expect(screen.getByTestId('tone-trigger')).toHaveTextContent('Direct');
  });

  it('forwards ...rest, data-testid and ref onto the trigger', () => {
    const ref = createRef<HTMLButtonElement>();
    render(<Harness triggerProps={{ id: 'tone-select', ref }} />);

    const trigger = screen.getByTestId('tone-trigger');
    expect(trigger).toHaveAttribute('id', 'tone-select');
    expect(ref.current).toBe(trigger);
  });

  it('exposes the size axis as data-size', () => {
    const { rerender } = render(<Harness />);
    expect(screen.getByTestId('tone-trigger')).toHaveAttribute('data-size', 'md');

    rerender(<Harness triggerProps={{ inputSize: 'sm' }} />);
    expect(screen.getByTestId('tone-trigger')).toHaveAttribute('data-size', 'sm');
  });

  /**
   * Keyboard only. A pointer open in jsdom depends on pointer capture and a
   * popper measurement pass over a zero-sized layout; the keyboard path is
   * both more stable and the one this primitive has to guarantee.
   */
  it('opens on Enter and commits the chosen option by keyboard', () => {
    const onValueChange = vi.fn();
    render(<Harness defaultValue="calm" onValueChange={onValueChange} />);

    const trigger = screen.getByTestId('tone-trigger');
    trigger.focus();
    fireEvent.keyDown(trigger, { key: 'Enter' });
    expect(trigger).toHaveAttribute('aria-expanded', 'true');

    // Enter on the option is the commit path Radix wires to SELECTION_KEYS.
    // Arrow-key highlight movement is not asserted here: it is implemented by
    // calling `.focus()` on the next candidate, and jsdom's focus does not
    // follow an element that layout never placed — so the assertion would be
    // testing jsdom, not the primitive.
    fireEvent.keyDown(screen.getByRole('option', { name: 'Direct' }), { key: 'Enter' });

    expect(onValueChange).toHaveBeenCalledWith('direct');
    expect(trigger).toHaveTextContent('Direct');
  });

  it('closes on Escape without changing the value', () => {
    const onValueChange = vi.fn();
    render(<Harness defaultValue="calm" onValueChange={onValueChange} />);

    const trigger = screen.getByTestId('tone-trigger');
    trigger.focus();
    fireEvent.keyDown(trigger, { key: 'ArrowDown' });
    expect(screen.getByRole('listbox')).toBeInTheDocument();

    fireEvent.keyDown(screen.getByRole('listbox'), { key: 'Escape' });
    expect(screen.queryByRole('listbox')).toBeNull();
    expect(onValueChange).not.toHaveBeenCalled();
  });

  it('does not open when disabled', () => {
    render(<Harness triggerProps={{ disabled: true }} />);

    const trigger = screen.getByTestId('tone-trigger');
    fireEvent.keyDown(trigger, { key: 'Enter' });
    expect(screen.queryByRole('listbox')).toBeNull();
  });

  it('lets a caller className win over the default trigger box', () => {
    render(<Harness triggerProps={{ className: 'h-12' }} />);
    const className = screen.getByTestId('tone-trigger').className;
    expect(className).toContain('h-12');
    expect(className).not.toContain('h-9');
  });

  it('never renders a raw palette colour on the trigger or the open content', () => {
    render(<Harness defaultValue="calm" />);

    const trigger = screen.getByTestId('tone-trigger');
    expect(trigger.className).not.toMatch(RAW_PALETTE);

    trigger.focus();
    fireEvent.keyDown(trigger, { key: 'Enter' });

    for (const el of Array.from(document.querySelectorAll('[data-slot^="select-"]'))) {
      expect(el.className, el.getAttribute('data-slot') ?? '').not.toMatch(RAW_PALETTE);
    }
  });
});
