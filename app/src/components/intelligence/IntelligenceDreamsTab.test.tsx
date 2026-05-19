import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import IntelligenceDreamsTab from './IntelligenceDreamsTab';

describe('<IntelligenceDreamsTab />', () => {
  it('renders the dreams title, description and coming-soon line', () => {
    render(<IntelligenceDreamsTab />);
    // useT resolves against the bundled English map by default.
    expect(screen.getByRole('heading', { level: 2, name: /^Dreams$/ })).toBeInTheDocument();
    expect(screen.getByText(/AI-generated reflections/i)).toBeInTheDocument();
    expect(screen.getByText(/Coming soon/i)).toBeInTheDocument();
  });

  it('renders a decorative svg icon', () => {
    const { container } = render(<IntelligenceDreamsTab />);
    expect(container.querySelector('svg')).not.toBeNull();
  });
});
