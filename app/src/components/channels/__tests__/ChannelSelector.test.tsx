import { fireEvent, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { FALLBACK_DEFINITIONS } from '../../../lib/channels/definitions';
import { renderWithProviders } from '../../../test/test-utils';
import ChannelSelector from '../ChannelSelector';

describe('ChannelSelector', () => {
  const onSelect = vi.fn();

  it('renders all channel tabs', () => {
    renderWithProviders(
      <ChannelSelector
        definitions={FALLBACK_DEFINITIONS}
        selectedChannel="telegram"
        onSelectChannel={onSelect}
      />
    );

    expect(screen.getByText('Telegram')).toBeInTheDocument();
    expect(screen.getByText('Discord')).toBeInTheDocument();
    expect(screen.getByText('Web')).toBeInTheDocument();
  });

  it('calls onSelectChannel when a tab is clicked', () => {
    renderWithProviders(
      <ChannelSelector
        definitions={FALLBACK_DEFINITIONS}
        selectedChannel="telegram"
        onSelectChannel={onSelect}
      />
    );

    fireEvent.click(screen.getByText('Discord'));
    expect(onSelect).toHaveBeenCalledWith('discord');
  });

  it('shows active route summary', () => {
    renderWithProviders(
      <ChannelSelector
        definitions={FALLBACK_DEFINITIONS}
        selectedChannel="telegram"
        onSelectChannel={onSelect}
      />
    );

    expect(screen.getByText(/No active route/)).toBeInTheDocument();
  });
});
