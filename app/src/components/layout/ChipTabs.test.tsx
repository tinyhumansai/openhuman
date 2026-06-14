import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import ChipTabs, { type ChipTabItem } from './ChipTabs';

type TabId = 'one' | 'two' | 'three';

const items: ChipTabItem<TabId>[] = [
  { id: 'one', label: 'One' },
  { id: 'two', label: 'Two' },
  { id: 'three', label: 'Three' },
];

describe('ChipTabs', () => {
  it('renders a tablist with one chip per item by default', () => {
    render(<ChipTabs items={items} value="one" onChange={() => {}} ariaLabel="Sections" />);

    const list = screen.getByRole('tablist', { name: 'Sections' });
    expect(list).toBeInTheDocument();
    expect(screen.getAllByRole('tab')).toHaveLength(3);
  });

  it('marks the active chip with aria-selected', () => {
    render(<ChipTabs items={items} value="two" onChange={() => {}} testIdPrefix="t" />);

    expect(screen.getByTestId('t-two')).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByTestId('t-one')).toHaveAttribute('aria-selected', 'false');
    expect(screen.getByTestId('t-three')).toHaveAttribute('aria-selected', 'false');
  });

  it('emits onChange with the clicked chip id', () => {
    const onChange = vi.fn();
    render(<ChipTabs items={items} value="one" onChange={onChange} testIdPrefix="t" />);

    fireEvent.click(screen.getByTestId('t-three'));
    expect(onChange).toHaveBeenCalledWith('three');
  });

  it('uses an explicit per-item testId over the prefix', () => {
    render(
      <ChipTabs
        items={[{ id: 'one', label: 'One', testId: 'custom-chip' }]}
        value="one"
        onChange={() => {}}
        testIdPrefix="t"
      />
    );

    expect(screen.getByTestId('custom-chip')).toBeInTheDocument();
    expect(screen.queryByTestId('t-one')).not.toBeInTheDocument();
  });

  it('renders navigation semantics with aria-current when as="nav"', () => {
    render(<ChipTabs items={items} value="two" onChange={() => {}} as="nav" ariaLabel="Sub nav" />);

    expect(screen.getByRole('navigation', { name: 'Sub nav' })).toBeInTheDocument();
    // No tab roles in nav mode.
    expect(screen.queryAllByRole('tab')).toHaveLength(0);
    expect(screen.getByText('Two')).toHaveAttribute('aria-current', 'page');
    expect(screen.getByText('One')).not.toHaveAttribute('aria-current');
  });
});
