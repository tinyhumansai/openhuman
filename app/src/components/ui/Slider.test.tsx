import { fireEvent, render, screen } from '@testing-library/react';
import { createRef } from 'react';
import { describe, expect, it, vi } from 'vitest';

import Slider from './Slider';

const RAW_PALETTE = /\b(?:bg|text|border|ring)-(?:neutral|stone|slate|canvas|white|black)\b/;

describe('Slider', () => {
  it('renders one thumb per value with slider semantics', () => {
    render(<Slider defaultValue={[40]} aria-label="Volume" data-testid="volume" />);

    const thumb = screen.getByRole('slider');
    expect(thumb).toHaveAttribute('aria-valuenow', '40');
    expect(screen.getByTestId('volume')).toHaveAttribute('data-slot', 'slider');
  });

  it('renders a thumb per entry for a two-value range', () => {
    render(<Slider defaultValue={[20, 70]} thumbLabels={['Min', 'Max']} />);

    const thumbs = screen.getAllByRole('slider');
    expect(thumbs).toHaveLength(2);
    expect(thumbs[0]).toHaveAccessibleName('Min');
    expect(thumbs[1]).toHaveAccessibleName('Max');
  });

  it('forwards ...rest, data-testid and ref onto the root', () => {
    const ref = createRef<HTMLSpanElement>();
    render(<Slider defaultValue={[10]} ref={ref} data-testid="slider" id="threshold" />);

    const root = screen.getByTestId('slider');
    expect(root).toHaveAttribute('id', 'threshold');
    expect(ref.current).toBe(root);
  });

  it('exposes the size axis as data-size', () => {
    const { rerender } = render(<Slider defaultValue={[10]} data-testid="s" />);
    expect(screen.getByTestId('s')).toHaveAttribute('data-size', 'md');

    rerender(<Slider defaultValue={[10]} size="sm" data-testid="s" />);
    expect(screen.getByTestId('s')).toHaveAttribute('data-size', 'sm');
  });

  /**
   * Keyboard, not pointer: jsdom has no layout, so a pointer drag resolves
   * every position to the same zero-width rect. Arrow keys are also the path a
   * keyboard user takes, which is the one this primitive exists to guarantee.
   */
  it('steps the value with ArrowRight / ArrowLeft', () => {
    const onValueChange = vi.fn();
    render(<Slider defaultValue={[50]} step={5} onValueChange={onValueChange} />);

    const thumb = screen.getByRole('slider');
    thumb.focus();

    fireEvent.keyDown(thumb, { key: 'ArrowRight' });
    expect(onValueChange).toHaveBeenLastCalledWith([55]);

    fireEvent.keyDown(thumb, { key: 'ArrowLeft' });
    fireEvent.keyDown(thumb, { key: 'ArrowLeft' });
    expect(onValueChange).toHaveBeenLastCalledWith([45]);
  });

  it('clamps at min and max rather than stepping past them', () => {
    const onValueChange = vi.fn();
    render(<Slider defaultValue={[100]} min={0} max={100} onValueChange={onValueChange} />);

    const thumb = screen.getByRole('slider');
    thumb.focus();
    fireEvent.keyDown(thumb, { key: 'ArrowRight' });

    expect(thumb).toHaveAttribute('aria-valuenow', '100');
  });

  it('does not respond to arrow keys when disabled', () => {
    const onValueChange = vi.fn();
    render(<Slider defaultValue={[50]} disabled onValueChange={onValueChange} />);

    const thumb = screen.getByRole('slider');
    fireEvent.keyDown(thumb, { key: 'ArrowRight' });
    expect(onValueChange).not.toHaveBeenCalled();
  });

  it('lets a caller className win over the default box', () => {
    render(<Slider defaultValue={[10]} className="w-40" data-testid="s" />);
    const className = screen.getByTestId('s').className;
    expect(className).toContain('w-40');
    expect(className).not.toContain('w-full');
  });

  it('never renders a raw palette colour in any size', () => {
    for (const size of ['sm', 'md'] as const) {
      const { unmount, container } = render(<Slider defaultValue={[30, 60]} size={size} />);
      for (const el of Array.from(container.querySelectorAll('[data-slot^="slider"]'))) {
        expect(el.className, `${size}/${el.getAttribute('data-slot')}`).not.toMatch(RAW_PALETTE);
      }
      unmount();
    }
  });
});
