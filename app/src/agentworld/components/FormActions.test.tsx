import { render, screen } from '@testing-library/react';
import { describe, expect, test } from 'vitest';

import FormActions from './FormActions';

describe('FormActions', () => {
  test('aligns actions to the end by default', () => {
    render(
      <FormActions>
        <button type="button">Save</button>
      </FormActions>
    );

    expect(screen.getByRole('button', { name: 'Save' }).parentElement).toHaveClass(
      'flex',
      'justify-end'
    );
  });

  test.each([
    ['start', 'justify-start'],
    ['end', 'justify-end'],
    ['stretch', 'items-stretch'],
  ] as const)('maps %s alignment to %s', (align, expectedClass) => {
    render(
      <FormActions align={align}>
        <button type="button">{align}</button>
      </FormActions>
    );

    expect(screen.getByRole('button', { name: align }).parentElement).toHaveClass(expectedClass);
  });

  test('appends a custom class', () => {
    render(
      <FormActions className="pt-4">
        <button type="button">Save</button>
      </FormActions>
    );

    expect(screen.getByRole('button', { name: 'Save' }).parentElement).toHaveClass('pt-4');
  });
});
