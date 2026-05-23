import { fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { FALLBACK_DEFINITIONS } from '../../../lib/channels/definitions';
import { channelConnectionsApi } from '../../../services/api/channelConnectionsApi';
import { renderWithProviders } from '../../../test/test-utils';
import { openUrl } from '../../../utils/openUrl';
import TelegramConfig from '../TelegramConfig';

const telegramDef = FALLBACK_DEFINITIONS.find(d => d.id === 'telegram')!;

vi.mock('../../../services/api/channelConnectionsApi', () => ({
  channelConnectionsApi: {
    connectChannel: vi.fn(),
    disconnectChannel: vi.fn(),
    listDefinitions: vi.fn(),
    listStatus: vi.fn(),
    telegramLoginStart: vi.fn(),
    telegramLoginCheck: vi.fn(),
  },
}));

vi.mock('../../../utils/openUrl', () => ({ openUrl: vi.fn() }));

afterEach(() => {
  vi.clearAllMocks();
});

describe('TelegramConfig', () => {
  it('renders auth mode labels', () => {
    renderWithProviders(<TelegramConfig definition={telegramDef} />);
    expect(screen.getByText('Login with OpenHuman')).toBeInTheDocument();
  });

  it('renders both auth modes', () => {
    renderWithProviders(<TelegramConfig definition={telegramDef} />);
    expect(screen.getAllByText(/Bot Token/i).length).toBeGreaterThanOrEqual(1);
    expect(screen.getByText('Login with OpenHuman')).toBeInTheDocument();
  });

  it('documents Telegram remote-control commands', () => {
    renderWithProviders(<TelegramConfig definition={telegramDef} />);
    expect(screen.getByText('Remote control (Telegram)')).toBeInTheDocument();
    expect(screen.getByText(/send \/status, \/sessions, \/new, or \/help/i)).toBeInTheDocument();
    expect(screen.getByText(/Model routing still uses \/model and \/models/i)).toBeInTheDocument();
  });

  it('shows credential fields for bot_token mode', () => {
    renderWithProviders(<TelegramConfig definition={telegramDef} />);
    expect(screen.getByPlaceholderText(/ABC-DEF1234/)).toBeInTheDocument();
    expect(screen.getByPlaceholderText(/Comma-separated/)).toBeInTheDocument();
  });

  it('shows Connect buttons for each auth mode', () => {
    renderWithProviders(<TelegramConfig definition={telegramDef} />);
    const connectButtons = screen.getAllByText('Connect');
    expect(connectButtons.length).toBe(2);
  });

  it('shows Disconnect buttons (disabled when disconnected)', () => {
    renderWithProviders(<TelegramConfig definition={telegramDef} />);
    const disconnectButtons = screen.getAllByText('Disconnect');
    expect(disconnectButtons.length).toBe(2);
    disconnectButtons.forEach(btn => {
      expect(btn).toBeDisabled();
    });
  });

  it('starts managed dm flow via core RPC, opens the deep link, and marks connected after polling', async () => {
    vi.mocked(channelConnectionsApi.connectChannel).mockResolvedValue({
      status: 'pending_auth',
      auth_action: 'telegram_managed_dm',
      restart_required: false,
    });
    vi.mocked(channelConnectionsApi.telegramLoginStart).mockResolvedValue({
      linkToken: 'link-token-abc',
      telegramUrl: 'https://t.me/openhuman_bot?start=link-token-abc',
      botUsername: 'openhuman_bot',
    });
    vi.mocked(channelConnectionsApi.telegramLoginCheck).mockResolvedValue({
      linked: true,
      details: { telegramUserId: '12345' },
    });

    renderWithProviders(<TelegramConfig definition={telegramDef} />);

    const connectButtons = screen.getAllByText('Connect');
    fireEvent.click(connectButtons[0]);

    await waitFor(() => {
      expect(channelConnectionsApi.telegramLoginStart).toHaveBeenCalledTimes(1);
    });
    await waitFor(() => {
      expect(openUrl).toHaveBeenCalledWith('https://t.me/openhuman_bot?start=link-token-abc');
    });
    await waitFor(() => {
      expect(channelConnectionsApi.telegramLoginCheck).toHaveBeenCalledWith('link-token-abc');
    });
    expect(await screen.findByText('Connected')).toBeInTheDocument();
  });

  it('shows disconnect confirmation with clearMemory checkbox and calls API on confirm', async () => {
    vi.mocked(channelConnectionsApi.disconnectChannel).mockResolvedValue(undefined);

    renderWithProviders(<TelegramConfig definition={telegramDef} />, {
      preloadedState: {
        channelConnections: {
          schemaVersion: 1,
          migrationCompleted: true,
          defaultMessagingChannel: 'telegram',
          connections: {
            telegram: {
              bot_token: {
                channel: 'telegram',
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

    // "Disconnect" button should be enabled when connected.
    const disconnectBtns = screen.getAllByText('Disconnect');
    // bot_token is the second Disconnect button (managed_dm is first, disconnected by default).
    const botTokenBtn = disconnectBtns[1];
    expect(botTokenBtn).not.toBeDisabled();

    // Click Disconnect → confirmation UI appears.
    fireEvent.click(botTokenBtn);
    expect(await screen.findByText('Yes, disconnect')).toBeInTheDocument();
    expect(
      screen.getByText('Also delete all memory ingested from this source (cannot be undone)')
    ).toBeInTheDocument();

    // Click cancel → confirmation dismisses.
    fireEvent.click(screen.getByText('Cancel'));
    await waitFor(() => {
      expect(screen.queryByText('Yes, disconnect')).not.toBeInTheDocument();
    });

    // Click Disconnect again (bot_token button), then confirm with checkbox.
    fireEvent.click(screen.getAllByText('Disconnect')[1]);
    const checkbox = screen.getByRole('checkbox');
    fireEvent.click(checkbox);
    fireEvent.click(await screen.findByText('Yes, disconnect'));

    await waitFor(() => {
      expect(channelConnectionsApi.disconnectChannel).toHaveBeenCalledWith(
        'telegram',
        'bot_token',
        true
      );
    });
  });
});
