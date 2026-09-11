import { fireEvent } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import type { FieldRequirement } from '../../types/channels';
import ChannelFieldInput from './ChannelFieldInput';

const textField: FieldRequirement = {
  key: 'api_key',
  label: 'API Key',
  field_type: 'string',
  required: true,
  placeholder: 'sk-...',
};

const boolField: FieldRequirement = {
  key: 'use_tls',
  label: 'Use TLS',
  field_type: 'boolean',
  required: false,
  placeholder: '',
};

describe('<ChannelFieldInput />', () => {
  it('renders a text input associated with its label and calls onChange', () => {
    const onChange = vi.fn();
    const { getByLabelText } = renderWithProviders(
      <ChannelFieldInput field={textField} value="" onChange={onChange} />
    );
    const input = getByLabelText(/API Key/) as HTMLInputElement;
    expect(input).not.toBeNull();
    expect(input.type).toBe('text');
    fireEvent.change(input, { target: { value: 'secret-value' } });
    expect(onChange).toHaveBeenCalledWith('secret-value');
  });

  it('renders a password input for secret fields', () => {
    const secretField: FieldRequirement = { ...textField, field_type: 'secret' };
    const { getByLabelText } = renderWithProviders(
      <ChannelFieldInput field={secretField} value="" onChange={vi.fn()} />
    );
    const input = getByLabelText(/API Key/) as HTMLInputElement;
    expect(input.type).toBe('password');
  });

  it('renders a checkbox for boolean fields and toggles onChange', () => {
    const onChange = vi.fn();
    const { getByLabelText } = renderWithProviders(
      <ChannelFieldInput field={boolField} value="false" onChange={onChange} />
    );
    const checkbox = getByLabelText(/Use TLS/) as HTMLInputElement;
    expect(checkbox.type).toBe('checkbox');
    expect(checkbox.checked).toBe(false);
    checkbox.click();
    expect(onChange).toHaveBeenCalledWith('true');
  });
});
