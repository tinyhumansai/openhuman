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
});
