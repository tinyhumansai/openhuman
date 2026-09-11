import { fireEvent, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import ColorTokenField from './ColorTokenField';

describe('ColorTokenField', () => {
  it('renders the label and the token key / hex description', () => {
    renderWithProviders(
      <ColorTokenField
        tokenKey="surface-canvas"
        label="Canvas"
        value="47 110 244"
        onChange={() => {}}
      />
    );

    expect(screen.getByText('Canvas')).toBeInTheDocument();
    // The token id and the hex used to be one interpolated description string
    // on the far side of the row from the swatch. They are separate spans beside
    // the swatch now, so they are asserted separately.
    expect(screen.getByText('--surface-canvas')).toBeInTheDocument();
    expect(screen.getByText('#2f6ef4')).toBeInTheDocument();
  });

  it('exposes the native colour swatch with the converted hex value and an accessible name', () => {
    renderWithProviders(
      <ColorTokenField
        tokenKey="surface-canvas"
        label="Canvas"
        value="47 110 244"
        onChange={() => {}}
      />
    );

    const swatch = screen.getByLabelText('Canvas') as HTMLInputElement;
    expect(swatch).toHaveAttribute('type', 'color');
    expect(swatch.value).toBe('#2f6ef4');
  });

  it('calls onChange with the channel triple when the swatch changes', () => {
    const onChange = vi.fn();
    renderWithProviders(
      <ColorTokenField
        tokenKey="surface-canvas"
        label="Canvas"
        value="47 110 244"
        onChange={onChange}
      />
    );

    const swatch = screen.getByLabelText('Canvas');
    fireEvent.change(swatch, { target: { value: '#ff0000' } });
    expect(onChange).toHaveBeenCalledWith('255 0 0');
  });

  it('disables the swatch when disabled is passed', () => {
    renderWithProviders(
      <ColorTokenField
        tokenKey="surface-canvas"
        label="Canvas"
        value="47 110 244"
        disabled
        onChange={() => {}}
      />
    );

    expect(screen.getByLabelText('Canvas')).toBeDisabled();
  });
});
