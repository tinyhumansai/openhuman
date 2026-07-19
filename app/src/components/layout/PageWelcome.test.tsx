import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import PageWelcome from './PageWelcome';

describe('PageWelcome', () => {
  it('renders the pitch, CTAs and feature cards', () => {
    render(
      <PageWelcome
        testId="welcome"
        icon="🔗"
        eyebrow="Connections"
        title="Everything in one place"
        description="Connect the tools your agent needs."
        ctas={[
          { label: 'Connect a channel', onClick: () => {}, testId: 'cta-1' },
          { label: 'Browse skills', onClick: () => {} },
        ]}
        featuresHeading="What you can do here"
        features={[
          { icon: '📥', title: 'Bring channels in', description: 'Link them fast.' },
          { icon: '🤖', title: 'Let your agent act', description: 'Full context.' },
        ]}
      />
    );

    expect(screen.getByTestId('welcome')).toBeInTheDocument();
    expect(screen.getByText('Everything in one place')).toBeInTheDocument();
    expect(screen.getByText('Connect the tools your agent needs.')).toBeInTheDocument();
    expect(screen.getByText('What you can do here')).toBeInTheDocument();
    expect(screen.getByText('Bring channels in')).toBeInTheDocument();
    expect(screen.getByText('Let your agent act')).toBeInTheDocument();
    expect(screen.getByTestId('cta-1')).toBeInTheDocument();
    expect(screen.getByText('Browse skills')).toBeInTheDocument();
  });

  it('fires the CTA onClick', () => {
    const onClick = vi.fn();
    render(
      <PageWelcome
        icon="🔗"
        title="Title"
        description="Body"
        ctas={[{ label: 'Get started', onClick, testId: 'cta-go' }]}
      />
    );

    fireEvent.click(screen.getByTestId('cta-go'));
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it('omits the feature section when no features are given', () => {
    render(<PageWelcome testId="w" icon="🔗" title="Title" description="Body" />);
    expect(screen.queryByText('What you can do here')).not.toBeInTheDocument();
  });
});
