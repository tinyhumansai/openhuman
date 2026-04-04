import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import type { ChannelConnectionStatus } from '../../../types/channels';
import ChannelStatusBadge from '../ChannelStatusBadge';

describe('ChannelStatusBadge', () => {
  const statuses: ChannelConnectionStatus[] = ['connected', 'connecting', 'disconnected', 'error'];

  it.each(statuses)('renders the correct label for "%s"', status => {
    render(<ChannelStatusBadge status={status} />);
    const labels: Record<ChannelConnectionStatus, string> = {
      connected: 'Connected',
      connecting: 'Connecting',
      disconnected: 'Disconnected',
      error: 'Error',
    };
    expect(screen.getByText(labels[status])).toBeInTheDocument();
  });

  it('applies custom className', () => {
    const { container } = render(<ChannelStatusBadge status="connected" className="extra-class" />);
    expect(container.firstChild).toHaveClass('extra-class');
  });
});
