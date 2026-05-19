import { fireEvent, screen, within } from '@testing-library/react';
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

  it('shows error status when a channel has a failed auth mode', () => {
    renderWithProviders(
      <ChannelSelector
        definitions={FALLBACK_DEFINITIONS}
        selectedChannel="telegram"
        onSelectChannel={onSelect}
      />,
      {
        preloadedState: {
          channelConnections: {
            schemaVersion: 1,
            migrationCompleted: true,
            defaultMessagingChannel: 'telegram',
            connections: {
              telegram: {
                managed_dm: undefined,
                oauth: undefined,
                bot_token: {
                  channel: 'telegram',
                  authMode: 'bot_token',
                  status: 'error',
                  selectedDefault: false,
                  lastError: 'Invalid bot token',
                  capabilities: [],
                  updatedAt: '2026-05-19T00:00:00.000Z',
                },
                api_key: undefined,
              },
              discord: {
                managed_dm: undefined,
                oauth: undefined,
                bot_token: undefined,
                api_key: undefined,
              },
              web: {
                managed_dm: undefined,
                oauth: undefined,
                bot_token: undefined,
                api_key: undefined,
              },
            },
          },
        },
      }
    );

    expect(
      within(screen.getByRole('button', { name: /Telegram/ })).getByText('Error')
    ).toBeInTheDocument();
  });
});
