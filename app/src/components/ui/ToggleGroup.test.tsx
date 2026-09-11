import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { createRef } from 'react';
import { describe, expect, it, vi } from 'vitest';

import { ToggleGroupItem, ToggleGroupRoot } from './ToggleGroup';

const RAW_PALETTE = /\b(bg|text|border|ring)-(neutral|stone|slate|canvas|white|black)\b/;

const renderSingle = (props: Record<string, unknown> = {}) =>
  render(
    <ToggleGroupRoot
      type="single"
      aria-label="Alignment"
      defaultValue="left"
      data-testid="root"
      {...props}>
      <ToggleGroupItem value="left" aria-label="Left" data-testid="left" />
      <ToggleGroupItem value="center" aria-label="Center" data-testid="center" />
      <ToggleGroupItem value="right" aria-label="Right" data-testid="right" />
    </ToggleGroupRoot>
  );

describe('ToggleGroup', () => {
  it('renders a labelled group of pressed-state buttons', () => {
    renderSingle();
    // Radix models `type="single"` as a radiogroup, not a generic group.
    expect(screen.getByRole('radiogroup', { name: 'Alignment' })).toHaveAttribute(
      'data-slot',
      'toggle-group'
    );
    expect(screen.getByTestId('left')).toHaveAttribute('data-state', 'on');
    expect(screen.getByTestId('center')).toHaveAttribute('data-state', 'off');
  });

  it('forwards ...rest and data-testid to the item element', () => {
    renderSingle();
    const item = screen.getByTestId('center');
    expect(item).toHaveAttribute('aria-label', 'Center');
    expect(item).toHaveAttribute('data-slot', 'toggle-group-item');
  });

  it('forwards a ref to the root', () => {
    const ref = createRef<HTMLDivElement>();
    render(
      <ToggleGroupRoot ref={ref} type="single" aria-label="Alignment">
        <ToggleGroupItem value="left" aria-label="Left" />
      </ToggleGroupRoot>
    );
    expect(ref.current).toBeInstanceOf(HTMLElement);
  });

  it('moves the roving tab stop with arrow keys', async () => {
    const user = userEvent.setup();
    renderSingle();
    screen.getByTestId('left').focus();
    await user.keyboard('{ArrowRight}');
    expect(screen.getByTestId('center')).toHaveFocus();
  });

  it('selects with the keyboard in single mode', async () => {
    const user = userEvent.setup();
    const onValueChange = vi.fn();
    renderSingle({ onValueChange });
    screen.getByTestId('center').focus();
    await user.keyboard('{Enter}');
    expect(onValueChange).toHaveBeenCalledWith('center');
    expect(screen.getByTestId('center')).toHaveAttribute('data-state', 'on');
  });

  it('keeps several items on in multiple mode', () => {
    render(
      <ToggleGroupRoot type="multiple" aria-label="Format" defaultValue={['bold', 'italic']}>
        <ToggleGroupItem value="bold" aria-label="Bold" data-testid="bold" />
        <ToggleGroupItem value="italic" aria-label="Italic" data-testid="italic" />
      </ToggleGroupRoot>
    );
    expect(screen.getByTestId('bold')).toHaveAttribute('data-state', 'on');
    expect(screen.getByTestId('italic')).toHaveAttribute('data-state', 'on');
  });

  it('inherits the group axes and lets an item override them', () => {
    render(
      <ToggleGroupRoot type="single" aria-label="Alignment" variant="secondary" size="lg">
        <ToggleGroupItem value="left" aria-label="Left" data-testid="left" />
        <ToggleGroupItem value="right" aria-label="Right" size="xs" data-testid="right" />
      </ToggleGroupRoot>
    );
    expect(screen.getByTestId('left')).toHaveAttribute('data-variant', 'secondary');
    expect(screen.getByTestId('left')).toHaveAttribute('data-size', 'lg');
    expect(screen.getByTestId('right')).toHaveAttribute('data-size', 'xs');
    expect(screen.getByTestId('right')).toHaveAttribute('data-variant', 'secondary');
  });

  it('lets a caller className win over the defaults', () => {
    render(
      <ToggleGroupRoot type="single" aria-label="Alignment" className="gap-6" data-testid="group">
        <ToggleGroupItem value="left" aria-label="Left" className="h-20" data-testid="left" />
      </ToggleGroupRoot>
    );
    expect(screen.getByTestId('group').className).toContain('gap-6');
    expect(screen.getByTestId('group').className).not.toContain('gap-1');
    expect(screen.getByTestId('left').className).toContain('h-20');
    expect(screen.getByTestId('left').className).not.toContain('h-9');
  });

  it('emits no raw palette utility', () => {
    renderSingle();
    expect(screen.getByTestId('root').className).not.toMatch(RAW_PALETTE);
    expect(screen.getByTestId('left').className).not.toMatch(RAW_PALETTE);
  });
});
