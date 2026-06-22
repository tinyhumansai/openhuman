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
    // Issue #3763: the allowlist must be settable in the connect UI.
    expect(screen.getByPlaceholderText(/Discord user IDs, or \* for everyone/)).toBeInTheDocument();
  });

  it('shows Connect buttons for each auth mode', () => {
    renderWithProviders(<DiscordConfig definition={discordDef} />);
    const connectButtons = screen.getAllByText('Connect');
    expect(connectButtons.length).toBe(3);
  });

  // #3794 review (Codex P2): clearing the allowlist must reach the backend as an
  // explicit empty value so it means "allow everyone", instead of being omitted
  // and silently reusing the previously-saved list on reconnect.
  it('submits an explicit empty allowed_users when the field is cleared', async () => {
    vi.mocked(channelConnectionsApi.connectChannel).mockResolvedValue({
      status: 'connected',
      restart_required: true,
    });
    renderWithProviders(<DiscordConfig definition={discordDef} />);

    fireEvent.change(screen.getByPlaceholderText(/Your Discord bot token/), {
      target: { value: 'bot-token-xyz' },
    });
    const allowlist = screen.getByPlaceholderText(/Discord user IDs, or \* for everyone/);
    fireEvent.change(allowlist, { target: { value: '111,222' } });
    fireEvent.change(allowlist, { target: { value: '' } }); // user clears it

    // bot_token is the first auth mode, so its Connect button is index 0.
    fireEvent.click(screen.getAllByRole('button', { name: 'Connect' })[0]);

    await waitFor(() => {
      expect(channelConnectionsApi.connectChannel).toHaveBeenCalledWith('discord', {
        authMode: 'bot_token',
        credentials: { bot_token: 'bot-token-xyz', allowed_users: '' },
      });
    });
  });

  it('omits allowed_users entirely when the field is never touched', async () => {
    vi.mocked(channelConnectionsApi.connectChannel).mockResolvedValue({
      status: 'connected',
      restart_required: true,
    });
    renderWithProviders(<DiscordConfig definition={discordDef} />);

    fireEvent.change(screen.getByPlaceholderText(/Your Discord bot token/), {
      target: { value: 'bot-token-xyz' },
    });

    fireEvent.click(screen.getAllByRole('button', { name: 'Connect' })[0]);

    await waitFor(() => {
      expect(channelConnectionsApi.connectChannel).toHaveBeenCalledWith('discord', {
        authMode: 'bot_token',
        credentials: { bot_token: 'bot-token-xyz' },
      });
    });
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
