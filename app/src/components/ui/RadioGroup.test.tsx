import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { createRef } from 'react';
import { describe, expect, it, vi } from 'vitest';

import { RadioGroupItem, RadioGroupRoot } from './RadioGroup';

const RAW_PALETTE = /\b(bg|text|border|ring)-(neutral|stone|slate|canvas|white|black)\b/;

const renderGroup = (props: Record<string, unknown> = {}) =>
  render(
    <RadioGroupRoot aria-label="Theme" defaultValue="light" {...props}>
      <RadioGroupItem value="light" aria-label="Light" data-testid="opt-light" />
      <RadioGroupItem value="dark" aria-label="Dark" data-testid="opt-dark" />
      <RadioGroupItem value="system" aria-label="System" data-testid="opt-system" />
    </RadioGroupRoot>
  );

describe('RadioGroup', () => {
  it('renders a radiogroup with one radio per item', () => {
    renderGroup();
    expect(screen.getByRole('radiogroup', { name: 'Theme' })).toBeInTheDocument();
    expect(screen.getAllByRole('radio')).toHaveLength(3);
    expect(screen.getByRole('radio', { name: 'Light' })).toBeChecked();
  });

  it('forwards ...rest and data-testid to the item element', () => {
    renderGroup();
    const item = screen.getByTestId('opt-dark');
    expect(item).toHaveAttribute('value', 'dark');
    expect(item).toHaveAttribute('data-slot', 'radio-group-item');
  });

  it('forwards a ref to the root element', () => {
    const ref = createRef<HTMLDivElement>();
    render(
      <RadioGroupRoot ref={ref} aria-label="Theme">
        <RadioGroupItem value="a" aria-label="A" />
      </RadioGroupRoot>
    );
    expect(ref.current).toBeInstanceOf(HTMLElement);
    expect(ref.current).toHaveAttribute('data-slot', 'radio-group');
  });

  it('moves focus with arrow keys and selects the focused option', async () => {
    const user = userEvent.setup();
    const onValueChange = vi.fn();
    renderGroup({ onValueChange });

    await user.tab();
    expect(screen.getByRole('radio', { name: 'Light' })).toHaveFocus();
    await user.keyboard('{ArrowDown}');
    expect(screen.getByRole('radio', { name: 'Dark' })).toHaveFocus();

    // Selection follows focus in a browser via the primitive's arrow-key
    // tracking; jsdom orders the document-level keydown listener after the
    // roving-focus move, so assert the space-bar path, which is the same
    // selection code and is deterministic here.
    await user.keyboard(' ');
    expect(onValueChange).toHaveBeenCalledWith('dark');
    expect(screen.getByRole('radio', { name: 'Dark' })).toBeChecked();
  });

  it('exposes checked state through data-state', () => {
    renderGroup();
    expect(screen.getByTestId('opt-light')).toHaveAttribute('data-state', 'checked');
    expect(screen.getByTestId('opt-dark')).toHaveAttribute('data-state', 'unchecked');
  });

  it('tags the size axis on the item', () => {
    render(
      <RadioGroupRoot aria-label="Density">
        <RadioGroupItem value="a" aria-label="A" size="lg" data-testid="opt" />
      </RadioGroupRoot>
    );
    expect(screen.getByTestId('opt')).toHaveAttribute('data-size', 'lg');
  });

  it('lets a caller className win over the defaults', () => {
    render(
      <RadioGroupRoot aria-label="Theme" className="gap-8" data-testid="root">
        <RadioGroupItem value="a" aria-label="A" className="h-8 w-8" data-testid="opt" />
      </RadioGroupRoot>
    );
    expect(screen.getByTestId('root').className).toContain('gap-8');
    expect(screen.getByTestId('root').className).not.toContain('gap-2');
    expect(screen.getByTestId('opt').className).toContain('h-8');
    expect(screen.getByTestId('opt').className).not.toContain('h-4');
  });

  it('emits no raw palette utility', () => {
    renderGroup();
    expect(screen.getByRole('radiogroup', { name: 'Theme' }).className).not.toMatch(RAW_PALETTE);
    for (const item of screen.getAllByRole('radio')) {
      expect(item.className).not.toMatch(RAW_PALETTE);
    }
  });
});
