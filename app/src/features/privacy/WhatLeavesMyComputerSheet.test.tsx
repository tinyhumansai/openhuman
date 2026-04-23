import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { WHAT_LEAVES_ITEMS } from './whatLeavesItems';
import WhatLeavesLink from './WhatLeavesLink';
import WhatLeavesMyComputerSheet from './WhatLeavesMyComputerSheet';

describe('WhatLeavesMyComputerSheet', () => {
  it('renders nothing when closed', () => {
    render(<WhatLeavesMyComputerSheet open={false} onClose={() => {}} />);
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('lists all five honest leave items when open', () => {
    render(<WhatLeavesMyComputerSheet open={true} onClose={() => {}} />);
    expect(screen.getByRole('dialog')).toBeInTheDocument();
    for (const item of WHAT_LEAVES_ITEMS) {
      expect(screen.getByText(item.title)).toBeInTheDocument();
    }
    expect(WHAT_LEAVES_ITEMS).toHaveLength(5);
  });

  it('calls onClose when Got it is clicked', () => {
    const onClose = vi.fn();
    render(<WhatLeavesMyComputerSheet open={true} onClose={onClose} />);
    fireEvent.click(screen.getByRole('button', { name: 'Got it' }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('calls onClose on Escape', () => {
    const onClose = vi.fn();
    render(<WhatLeavesMyComputerSheet open={true} onClose={onClose} />);
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});

describe('WhatLeavesLink', () => {
  it('opens the sheet when clicked', () => {
    render(<WhatLeavesLink />);
    expect(screen.queryByRole('dialog')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'What leaves my computer?' }));
    expect(screen.getByRole('dialog')).toBeInTheDocument();
  });

  it('accepts a custom label', () => {
    render(<WhatLeavesLink label="Network activity?" />);
    expect(screen.getByRole('button', { name: 'Network activity?' })).toBeInTheDocument();
  });
});
