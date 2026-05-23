import { fireEvent, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { FALLBACK_DEFINITIONS } from '../../../lib/channels/definitions';
import { channelConnectionsApi } from '../../../services/api/channelConnectionsApi';
import { renderWithProviders } from '../../../test/test-utils';
import DiscordConfig from '../DiscordConfig';

vi.mock('../../../services/api/channelConnectionsApi', () => ({
  channelConnectionsApi: {
    connectChannel: vi.fn(),
    disconnectChannel: vi.fn(),
    listDefinitions: vi.fn(),
    listStatus: vi.fn(),
    discordLinkStart: vi.fn(),
    discordLinkCheck: vi.fn(),
  },
}));

const discordDef = FALLBACK_DEFINITIONS.find(d => d.id === 'discord')!;

describe('DiscordConfig', () => {
  it('renders auth mode labels', () => {
    renderWithProviders(<DiscordConfig definition={discordDef} />);
    expect(screen.getByText('OAuth Sign-in')).toBeInTheDocument();
  });

  it('renders both auth modes', () => {
    renderWithProviders(<DiscordConfig definition={discordDef} />);
    expect(screen.getAllByText('Bot Token').length).toBeGreaterThanOrEqual(1);
    expect(screen.getByText('OAuth Sign-in')).toBeInTheDocument();
  });

  it('shows credential fields for bot_token mode', () => {
    renderWithProviders(<DiscordConfig definition={discordDef} />);
    expect(screen.getByPlaceholderText(/Your Discord bot token/)).toBeInTheDocument();
    expect(screen.getByPlaceholderText(/restrict to a specific server/)).toBeInTheDocument();
  });

  it('shows Connect buttons for each auth mode', () => {
    renderWithProviders(<DiscordConfig definition={discordDef} />);
    const connectButtons = screen.getAllByText('Connect');
    expect(connectButtons.length).toBe(3);
  });

  it('shows disconnect confirmation with clearMemory checkbox and calls API on confirm', async () => {
    vi.mocked(channelConnectionsApi.disconnectChannel).mockResolvedValue(undefined);

    renderWithProviders(<DiscordConfig definition={discordDef} />, {
      preloadedState: {
        channelConnections: {
          schemaVersion: 1,
          migrationCompleted: true,
          defaultMessagingChannel: 'telegram',
          connections: {
            discord: {
              bot_token: {
                channel: 'discord',
                authMode: 'bot_token',
                status: 'connected',
                selectedDefault: false,
                capabilities: [],
                updatedAt: new Date().toISOString(),
              },
            },
          },
        },
      },
    });

    const disconnectBtns = screen.getAllByText('Disconnect');
    // Find the first enabled Disconnect button (bot_token).
    const enabledBtn = disconnectBtns.find(btn => !(btn as HTMLButtonElement).disabled);
    if (!enabledBtn) throw new Error('No enabled Disconnect button found');
    fireEvent.click(enabledBtn);
    expect(await screen.findByText('Yes, disconnect')).toBeInTheDocument();
    expect(
      screen.getByText('Also delete all memory ingested from this source (cannot be undone)')
    ).toBeInTheDocument();

    const checkbox = screen.getByRole('checkbox');
    fireEvent.click(checkbox);
    fireEvent.click(screen.getByText('Yes, disconnect'));

    await waitFor(() => {
      expect(channelConnectionsApi.disconnectChannel).toHaveBeenCalledWith(
        'discord',
        'bot_token',
        true
      );
    });
  });
});
