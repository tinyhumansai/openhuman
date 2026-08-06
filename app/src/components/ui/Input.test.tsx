import { render, screen } from '@testing-library/react';
import { createRef } from 'react';
import { describe, expect, it } from 'vitest';

import Input from './Input';

describe('Input', () => {
  it('forwards native input attributes and refs', () => {
    const ref = createRef<HTMLInputElement>();
    render(<Input ref={ref} aria-label="Name" placeholder="Ada" />);

    const input = screen.getByRole('textbox', { name: 'Name' });
    expect(input).toHaveAttribute('placeholder', 'Ada');
    expect(ref.current).toBe(input);
  });

  it('sets aria-invalid only for invalid inputs', () => {
    const { rerender } = render(<Input aria-label="Amount" invalid />);
    expect(screen.getByRole('textbox', { name: 'Amount' })).toHaveAttribute('aria-invalid', 'true');

    rerender(<Input aria-label="Amount" />);
    expect(screen.getByRole('textbox', { name: 'Amount' })).not.toHaveAttribute('aria-invalid');
  });

  it('adds monospace presentation only when requested', () => {
    const { rerender } = render(<Input aria-label="Token" monospace />);
    expect(screen.getByRole('textbox', { name: 'Token' })).toHaveClass('font-mono');

    rerender(<Input aria-label="Token" />);
    expect(screen.getByRole('textbox', { name: 'Token' })).not.toHaveClass('font-mono');
  });

  it('retains the configured input size', () => {
    render(<Input aria-label="Large input" inputSize="lg" />);
    expect(screen.getByRole('textbox', { name: 'Large input' })).toHaveClass('h-11');
  });
});
