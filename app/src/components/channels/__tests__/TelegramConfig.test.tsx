import { fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { FALLBACK_DEFINITIONS } from '../../../lib/channels/definitions';
import { channelConnectionsApi } from '../../../services/api/channelConnectionsApi';
import { upsertChannelConnection } from '../../../store/channelConnectionsSlice';
import { createTestStore, renderWithProviders } from '../../../test/test-utils';
import { openUrl } from '../../../utils/openUrl';
import TelegramConfig from '../TelegramConfig';

const telegramDef = FALLBACK_DEFINITIONS.find(d => d.id === 'telegram')!;
const coreStateMock = vi.hoisted(() => vi.fn(() => ({ snapshot: { sessionToken: 'jwt-abc' } })));

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
vi.mock('../../../providers/CoreStateProvider', () => ({ useCoreState: () => coreStateMock() }));

afterEach(() => {
  vi.clearAllMocks();
  coreStateMock.mockReturnValue({ snapshot: { sessionToken: 'jwt-abc' } });
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

  it('passes clearMemory when disconnecting with the memory checkbox selected', async () => {
    const store = createTestStore();
    store.dispatch(
      upsertChannelConnection({
        channel: 'telegram',
        authMode: 'bot_token',
        patch: { status: 'connected', capabilities: ['read', 'write'] },
      })
    );
    vi.mocked(channelConnectionsApi.disconnectChannel).mockResolvedValue(undefined);

    renderWithProviders(<TelegramConfig definition={telegramDef} />, { store });

    fireEvent.click(screen.getByLabelText(/also delete memory/i));
    const disconnectButton = screen
      .getAllByRole('button', { name: 'Disconnect' })
      .find(button => !button.hasAttribute('disabled'));
    expect(disconnectButton).toBeDefined();
    fireEvent.click(disconnectButton!);

    await waitFor(() => {
      expect(channelConnectionsApi.disconnectChannel).toHaveBeenCalledWith(
        'telegram',
        'bot_token',
        { clearMemory: true }
      );
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

  it('hides managed channel auth modes for local users', () => {
    coreStateMock.mockReturnValue({ snapshot: { sessionToken: 'header.payload.local' } });

    renderWithProviders(<TelegramConfig definition={telegramDef} />);

    expect(
      screen.getByText('Managed channels are not available for local users.')
    ).toBeInTheDocument();
    expect(screen.queryByText('Login with OpenHuman')).not.toBeInTheDocument();
    expect(screen.getAllByText(/Bot Token/i).length).toBeGreaterThanOrEqual(1);
  });
});
