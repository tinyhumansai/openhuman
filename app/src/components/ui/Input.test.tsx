import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import Input from './Input';

/**
 * These pin the ONE thing this component gets wrong when it is written the
 * obvious way: composing its class list by concatenation.
 *
 * Concatenation leaves the size defaults in the attribute beside the caller's
 * override and lets Tailwind's stylesheet order pick the winner, so an
 * override applies or not depending on where the two utilities happen to sit
 * in the generated CSS. `cn` (tailwind-merge) resolves the conflict last-wins,
 * which is what every caller assumes.
 */
describe('Input class composition', () => {
  it('lets a caller override the size preset padding', () => {
    render(<Input inputSize="sm" className="px-2" aria-label="field" />);
    const el = screen.getByLabelText('field');
    // `px-2.5` is `SIZES.sm`'s padding. Under concatenation both survived and
    // `px-2.5` won on stylesheet order, silently ignoring the caller.
    expect(el).toHaveClass('px-2');
    expect(el).not.toHaveClass('px-2.5');
  });

  it('lets a caller override the size preset font size and height', () => {
    render(<Input inputSize="sm" className="h-auto text-2xl" aria-label="heading" />);
    const el = screen.getByLabelText('heading');
    expect(el).toHaveClass('h-auto', 'text-2xl');
    expect(el).not.toHaveClass('h-8');
    expect(el).not.toHaveClass('text-sm');
  });

  it('keeps the defaults a caller does not override', () => {
    render(<Input inputSize="sm" className="px-2" aria-label="field" />);
    const el = screen.getByLabelText('field');
    expect(el).toHaveClass('h-8', 'text-sm', 'rounded-md', 'w-full', 'border');
  });

  it('still applies the invalid ring and monospace flags', () => {
    render(<Input invalid monospace aria-label="bad" />);
    const el = screen.getByLabelText('bad');
    expect(el).toHaveClass('font-mono', 'border-coral-400');
    expect(el).toHaveAttribute('aria-invalid', 'true');
  });
});
