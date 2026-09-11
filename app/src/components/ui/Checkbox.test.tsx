import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';

import Checkbox from './Checkbox';

describe('<Checkbox />', () => {
  it('keeps the native checked state in sync when toggled', async () => {
    function ControlledCheckbox() {
      const [checked, setChecked] = useState(false);
      return <Checkbox aria-label="Source" checked={checked} onCheckedChange={setChecked} />;
    }
    render(<ControlledCheckbox />);
    const checkbox = screen.getByRole('checkbox', { name: 'Source' });
    expect(checkbox).not.toBeChecked();
    await userEvent.click(checkbox);
    expect(checkbox).toBeChecked();
    await userEvent.click(checkbox);
    expect(checkbox).not.toBeChecked();
  });

  it('updates and clears the native indeterminate state', () => {
    const onCheckedChange = vi.fn();
    const { rerender } = render(
      <Checkbox checked={false} indeterminate onCheckedChange={onCheckedChange} />
    );
    expect(screen.getByRole('checkbox')).toBePartiallyChecked();
    rerender(<Checkbox checked={false} indeterminate={false} onCheckedChange={onCheckedChange} />);
    expect(screen.getByRole('checkbox')).not.toBePartiallyChecked();
  });

  it('does not toggle a disabled checkbox', async () => {
    const onCheckedChange = vi.fn();
    render(<Checkbox checked disabled onCheckedChange={onCheckedChange} />);
    await userEvent.click(screen.getByRole('checkbox'));
    expect(screen.getByRole('checkbox')).toBeChecked();
    expect(onCheckedChange).not.toHaveBeenCalled();
  });
});
