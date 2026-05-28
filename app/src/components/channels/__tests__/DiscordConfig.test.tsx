import { fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { FALLBACK_DEFINITIONS } from '../../../lib/channels/definitions';
import { channelConnectionsApi } from '../../../services/api/channelConnectionsApi';
import { upsertChannelConnection } from '../../../store/channelConnectionsSlice';
import { createTestStore, renderWithProviders } from '../../../test/test-utils';
import DiscordConfig from '../DiscordConfig';

const coreStateMock = vi.hoisted(() => vi.fn(() => ({ snapshot: { sessionToken: 'jwt-abc' } })));

vi.mock('../../../providers/CoreStateProvider', () => ({ useCoreState: () => coreStateMock() }));

const discordDef = FALLBACK_DEFINITIONS.find(d => d.id === 'discord')!;

vi.mock('../../../hooks/useOAuthConnectionListener', () => ({
  useOAuthConnectionListener: vi.fn(),
}));

vi.mock('../../../services/api/channelConnectionsApi', () => ({
  channelConnectionsApi: {
    connectChannel: vi.fn(),
    disconnectChannel: vi.fn(),
    discordLinkStart: vi.fn(),
    discordLinkCheck: vi.fn(),
    listDefinitions: vi.fn(),
    listStatus: vi.fn(),
  },
}));

vi.mock('../../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

vi.mock('../../../utils/openUrl', () => ({ openUrl: vi.fn() }));

vi.mock('../../../utils/tauriCommands/core', () => ({ restartCoreProcess: vi.fn() }));

afterEach(() => {
  vi.clearAllMocks();
  coreStateMock.mockReturnValue({ snapshot: { sessionToken: 'jwt-abc' } });
});

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

  it('passes clearMemory when disconnecting a connected bot token account', async () => {
    const store = createTestStore();
    store.dispatch(
      upsertChannelConnection({
        channel: 'discord',
        authMode: 'bot_token',
        patch: { status: 'connected', capabilities: ['read', 'write'] },
      })
    );
    vi.mocked(channelConnectionsApi.disconnectChannel).mockResolvedValue(undefined);

    renderWithProviders(<DiscordConfig definition={discordDef} />, { store });

    fireEvent.click(screen.getByLabelText(/also delete memory/i));
    const disconnectButton = screen
      .getAllByRole('button', { name: 'Disconnect' })
      .find(button => !button.hasAttribute('disabled'));
    expect(disconnectButton).toBeDefined();
    fireEvent.click(disconnectButton!);

    await waitFor(() => {
      expect(channelConnectionsApi.disconnectChannel).toHaveBeenCalledWith('discord', 'bot_token', {
        clearMemory: true,
      });
    });
  });

  it('passes clearMemory when disconnecting a connected managed DM account', async () => {
    const store = createTestStore();
    store.dispatch(
      upsertChannelConnection({
        channel: 'discord',
        authMode: 'managed_dm',
        patch: { status: 'connected', capabilities: ['dm'] },
      })
    );
    vi.mocked(channelConnectionsApi.disconnectChannel).mockResolvedValue(undefined);

    renderWithProviders(<DiscordConfig definition={discordDef} />, { store });

    fireEvent.click(screen.getByLabelText(/also delete memory/i));
    const disconnectButton = screen
      .getAllByRole('button', { name: 'Disconnect' })
      .find(button => !button.hasAttribute('disabled'));
    expect(disconnectButton).toBeDefined();
    fireEvent.click(disconnectButton!);

    await waitFor(() => {
      expect(channelConnectionsApi.disconnectChannel).toHaveBeenCalledWith(
        'discord',
        'managed_dm',
        { clearMemory: true }
      );
    });
  });

  it('hides managed channel auth modes for local users', () => {
    coreStateMock.mockReturnValue({ snapshot: { sessionToken: 'header.payload.local' } });

    renderWithProviders(<DiscordConfig definition={discordDef} />);

    expect(
      screen.getByText('Managed channels are not available for local users.')
    ).toBeInTheDocument();
    expect(screen.queryByText('OAuth Sign-in')).not.toBeInTheDocument();
    expect(screen.queryByText('Login with OpenHuman')).not.toBeInTheDocument();
    expect(screen.getAllByText('Bot Token').length).toBeGreaterThanOrEqual(1);
  });
});
