import { render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { MemoryEmptyPlaceholder } from './MemoryEmptyPlaceholder';

describe('<MemoryEmptyPlaceholder />', () => {
  it('renders the empty title and hint copy inside the testid root', () => {
    render(<MemoryEmptyPlaceholder />);
    const root = screen.getByTestId('memory-empty-placeholder');
    // useT resolves against the bundled English map by default.
    expect(
      within(root).getByRole('heading', { level: 2, name: /No memories yet/i })
    ).toBeInTheDocument();
    expect(
      within(root).getByText(/Start interacting to create your first memories/i)
    ).toBeInTheDocument();
  });
});
