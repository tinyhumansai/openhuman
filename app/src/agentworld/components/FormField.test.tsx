import { render, screen } from '@testing-library/react';
import { describe, expect, test } from 'vitest';

import FormField from './FormField';

describe('FormField', () => {
  test('associates its label, description, and error with the control', () => {
    render(
      <FormField
        id="profile-name"
        label="Name"
        description="Use your public name."
        error="Name is required."
        required>
        <input />
      </FormField>
    );

    const control = screen.getByRole('textbox', { name: 'Name' });
    expect(control).toHaveAttribute('id', 'profile-name');
    expect(control).toHaveAttribute(
      'aria-describedby',
      'profile-name-description profile-name-error'
    );
    expect(control).toHaveAttribute('aria-invalid', 'true');
    expect(control).toBeRequired();
    expect(screen.getByText('Use your public name.')).toHaveAttribute(
      'id',
      'profile-name-description'
    );
    expect(screen.getByRole('alert')).toHaveAttribute('id', 'profile-name-error');
  });

  test('does not overwrite accessibility props explicitly set on the child', () => {
    render(
      <FormField
        id="profile-email"
        label="Email"
        description="We only use this for notifications."
        error="Email is invalid."
        required>
        <input
          id="custom-email"
          aria-describedby="custom-description"
          aria-invalid={false}
          required={false}
        />
      </FormField>
    );

    const control = screen.getByRole('textbox');
    expect(control).toHaveAttribute('id', 'custom-email');
    expect(control).toHaveAttribute('aria-describedby', 'custom-description');
    expect(control).toHaveAttribute('aria-invalid', 'false');
    expect(control).not.toBeRequired();
  });

  test('omits description and error markup when neither is provided', () => {
    render(
      <FormField id="profile-handle" label="Handle">
        <input />
      </FormField>
    );

    const control = screen.getByRole('textbox', { name: 'Handle' });
    expect(control).not.toHaveAttribute('aria-describedby');
    expect(control).toHaveAttribute('aria-invalid', 'false');
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  test('appends a custom wrapper class', () => {
    const { container } = render(
      <FormField id="profile-bio" label="Bio" className="custom-field">
        <textarea />
      </FormField>
    );

    expect(container.firstChild).toHaveClass('custom-field');
  });
});
