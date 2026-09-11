/**
 * SkillSearchBar — smoke coverage for the ui/ primitive migration.
 *
 * The bar is a controlled `TextField` plus a clear `Button` that only exists
 * while there is something to clear. Both behaviours are asserted through
 * data-slot / accessible name rather than class strings.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import SkillSearchBar from './SkillSearchBar';

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

describe('SkillSearchBar', () => {
  it('raises each keystroke through onChange', () => {
    const onChange = vi.fn();
    render(<SkillSearchBar value="" onChange={onChange} placeholder="Search skills" />);

    const input = screen.getByPlaceholderText('Search skills');
    fireEvent.change(input, { target: { value: 'weather' } });
    expect(onChange).toHaveBeenCalledWith('weather');
  });

  it('hides the clear button while empty and clears to an empty string once filled', () => {
    const onChange = vi.fn();
    const { rerender } = render(<SkillSearchBar value="" onChange={onChange} />);
    expect(screen.queryByRole('button', { name: 'common.clear' })).not.toBeInTheDocument();

    rerender(<SkillSearchBar value="weather" onChange={onChange} />);
    const clear = screen.getByRole('button', { name: 'common.clear' });
    expect(clear).toHaveAttribute('data-slot', 'button');
    expect(clear).toHaveAttribute('data-variant', 'tertiary');

    fireEvent.click(clear);
    expect(onChange).toHaveBeenCalledWith('');
  });
});
