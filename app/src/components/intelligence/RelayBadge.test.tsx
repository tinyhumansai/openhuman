import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import RelayBadge from './RelayBadge';

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

describe('RelayBadge', () => {
  it('renders nothing when relay info is absent', () => {
    const { container } = render(<RelayBadge relay={null} />);
    expect(container.firstChild).toBeNull();
  });

  it('shows the staging label + base url and tags the network', () => {
    render(
      <RelayBadge relay={{ baseUrl: 'https://staging-api.tiny.place', network: 'staging' }} />
    );
    const badge = screen.getByTestId('tinyplace-relay-badge');
    expect(badge).toHaveAttribute('data-network', 'staging');
    expect(badge).toHaveAttribute('title', 'https://staging-api.tiny.place');
    expect(badge).toHaveTextContent('tinyplaceOrchestration.relay.staging');
  });

  it('shows the production label for the prod relay', () => {
    render(<RelayBadge relay={{ baseUrl: 'https://api.tiny.place', network: 'prod' }} />);
    const badge = screen.getByTestId('tinyplace-relay-badge');
    expect(badge).toHaveAttribute('data-network', 'prod');
    expect(badge).toHaveTextContent('tinyplaceOrchestration.relay.prod');
  });
});
