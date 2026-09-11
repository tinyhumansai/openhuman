import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import TwoPaneNav from './TwoPaneNav';

const groups = [
  { label: 'Group A', items: [{ value: 'alpha', label: 'Alpha' }] },
  { items: [{ value: 'beta', label: 'Beta' }] },
];

describe('TwoPaneNav', () => {
  it('renders one row per item, on the shared Button primitive', () => {
    render(<TwoPaneNav groups={groups} selected="alpha" onSelect={() => {}} ariaLabel="Panes" />);

    expect(screen.getByRole('navigation', { name: 'Panes' })).toBeInTheDocument();

    const alpha = screen.getByTestId('two-pane-nav-alpha');
    expect(alpha).toHaveAttribute('data-slot', 'button');
    expect(alpha).toHaveAttribute('data-variant', 'tertiary');
    expect(screen.getByTestId('two-pane-nav-beta')).toBeInTheDocument();
    expect(screen.getByText('Group A')).toBeInTheDocument();
  });

  it('marks only the selected row with aria-current', () => {
    render(<TwoPaneNav groups={groups} selected="beta" onSelect={() => {}} />);

    expect(screen.getByTestId('two-pane-nav-beta')).toHaveAttribute('aria-current', 'page');
    expect(screen.getByTestId('two-pane-nav-alpha')).not.toHaveAttribute('aria-current');
  });

  it('emits the item value on click', () => {
    const onSelect = vi.fn();
    render(<TwoPaneNav groups={groups} selected="alpha" onSelect={onSelect} />);

    fireEvent.click(screen.getByTestId('two-pane-nav-beta'));

    expect(onSelect).toHaveBeenCalledWith('beta');
  });

  it('renders the optional header and footer slots', () => {
    render(
      <TwoPaneNav
        groups={groups}
        selected="alpha"
        onSelect={() => {}}
        header={<span>Header slot</span>}
        footer={<span>Footer slot</span>}
      />
    );

    expect(screen.getByText('Header slot')).toBeInTheDocument();
    expect(screen.getByText('Footer slot')).toBeInTheDocument();
  });
});
