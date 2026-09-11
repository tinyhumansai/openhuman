import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import PersonaTemplatePicker from './PersonaTemplatePicker';

// Pass-through translator so assertions can target the i18n keys directly.
vi.mock('../../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

describe('<PersonaTemplatePicker />', () => {
  it('renders one button per persona template', () => {
    render(<PersonaTemplatePicker value="" onChange={vi.fn()} />);
    const buttons = screen
      .getAllByRole('button')
      .filter(b => b.getAttribute('data-testid')?.startsWith('persona-template-'));
    expect(buttons.length).toBeGreaterThan(0);
    buttons.forEach(button => {
      expect(button).toHaveAttribute('data-slot', 'button');
    });
  });

  it('applies the clicked template to the current value', () => {
    const onChange = vi.fn();
    render(<PersonaTemplatePicker value="" onChange={onChange} />);
    fireEvent.click(screen.getByTestId('persona-template-doctor'));
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange).toHaveBeenCalledWith(expect.any(String));
  });

  it('disables every template button when disabled', () => {
    render(<PersonaTemplatePicker value="" onChange={vi.fn()} disabled />);
    const buttons = screen
      .getAllByRole('button')
      .filter(b => b.getAttribute('data-testid')?.startsWith('persona-template-'));
    buttons.forEach(button => expect(button).toBeDisabled());
  });
});
