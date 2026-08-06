import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import StatusBlock from './StatusBlock';

describe('StatusBlock', () => {
  it.each([
    ['neutral', 'text-content-muted'],
    ['info', 'text-ocean-600'],
    ['success', 'text-sage-600'],
    ['warning', 'text-amber-600'],
    ['danger', 'text-coral-500'],
  ] as const)('maps the %s tone to semantic presentation classes', (tone, toneClass) => {
    render(<StatusBlock tone={tone} title={`${tone} title`} />);

    expect(screen.getByText(`${tone} title`)).toHaveClass(toneClass);
  });

  it('defaults to neutral and renders optional body and action content', () => {
    render(
      <StatusBlock
        title="Nothing here"
        body={<span>Try another filter.</span>}
        action={<button type="button">Reset</button>}
      />
    );

    const block = screen.getByTestId('agentworld-status-block');
    expect(block).toHaveClass('h-64');
    expect(screen.getByText('Nothing here')).toHaveClass('text-content-muted');
    expect(block).toContainElement(screen.getByText('Try another filter.'));
    expect(block).toContainElement(screen.getByRole('button', { name: 'Reset' }));
  });

  it('omits body and action wrappers when their content is absent', () => {
    render(<StatusBlock title="Ready" />);

    const block = screen.getByTestId('agentworld-status-block');
    expect(block.children).toHaveLength(1);
  });

  it('exposes its loading state and an accessible spinner', () => {
    render(<StatusBlock loading title="Loading agents" />);

    const block = screen.getByTestId('agentworld-status-block');
    expect(block).toHaveAttribute('aria-busy', 'true');
    expect(screen.getByRole('status', { name: 'Loading' })).toBeInTheDocument();
    expect(screen.getByText('Loading agents')).toBeInTheDocument();
  });
});
