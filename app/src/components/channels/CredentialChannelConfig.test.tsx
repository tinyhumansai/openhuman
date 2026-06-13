import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { FALLBACK_DEFINITIONS } from '../../lib/channels/definitions';
import { channelConnectionsApi } from '../../services/api/channelConnectionsApi';
import { renderWithProviders } from '../../test/test-utils';
import { restartCoreProcess } from '../../utils/tauriCommands/core';
import CredentialChannelConfig from './CredentialChannelConfig';

vi.mock('../../services/api/channelConnectionsApi', () => ({
  channelConnectionsApi: { connectChannel: vi.fn(), disconnectChannel: vi.fn() },
}));

vi.mock('../../utils/tauriCommands/core', () => ({ restartCoreProcess: vi.fn() }));

const larkDefinition = FALLBACK_DEFINITIONS.find(def => def.id === 'lark')!;
const dingtalkDefinition = FALLBACK_DEFINITIONS.find(def => def.id === 'dingtalk')!;

const connectChannelMock = vi.mocked(channelConnectionsApi.connectChannel);
const disconnectChannelMock = vi.mocked(channelConnectionsApi.disconnectChannel);
const restartCoreProcessMock = vi.mocked(restartCoreProcess);

beforeEach(() => {
  vi.clearAllMocks();
  connectChannelMock.mockResolvedValue({ status: 'connected', restart_required: true });
  restartCoreProcessMock.mockResolvedValue(undefined as never);
});

describe('<CredentialChannelConfig />', () => {
  it('renders the credential fields declared by the definition', () => {
    renderWithProviders(<CredentialChannelConfig definition={larkDefinition} />);
    expect(screen.getByPlaceholderText('cli_xxxxxxxxxxxx')).toBeInTheDocument(); // app_id
    expect(screen.getByPlaceholderText('Your Lark app secret')).toBeInTheDocument(); // app_secret
    // boolean field renders a checkbox, not a text input
    expect(screen.getByRole('checkbox')).toBeInTheDocument(); // use_feishu
  });

  it('connects with the entered credentials and restarts the core', async () => {
    renderWithProviders(<CredentialChannelConfig definition={larkDefinition} />);

    fireEvent.change(screen.getByPlaceholderText('cli_xxxxxxxxxxxx'), {
      target: { value: 'cli_abc123' },
    });
    fireEvent.change(screen.getByPlaceholderText('Your Lark app secret'), {
      target: { value: 'shh-secret' },
    });
    fireEvent.click(screen.getByText('Connect'));

    await waitFor(() => expect(connectChannelMock).toHaveBeenCalledTimes(1));
    expect(connectChannelMock).toHaveBeenCalledWith('lark', {
      authMode: 'api_key',
      credentials: { app_id: 'cli_abc123', app_secret: 'shh-secret' },
    });
    await waitFor(() => expect(restartCoreProcessMock).toHaveBeenCalledTimes(1));
  });

  it('does not mark the channel connected when the core restart fails', async () => {
    connectChannelMock.mockResolvedValue({ status: 'connected', restart_required: true });
    restartCoreProcessMock.mockRejectedValue(new Error('restart failed'));
    renderWithProviders(<CredentialChannelConfig definition={larkDefinition} />);

    fireEvent.change(screen.getByPlaceholderText('cli_xxxxxxxxxxxx'), {
      target: { value: 'cli_abc123' },
    });
    fireEvent.change(screen.getByPlaceholderText('Your Lark app secret'), {
      target: { value: 'shh-secret' },
    });
    fireEvent.click(screen.getByText('Connect'));

    // Restart failed → surfaces the saved-restart message and stays not-connected
    // (Connect button still present), rather than falsely marking connected.
    await waitFor(() =>
      expect(screen.getByText(/Restart the app to activate it/i)).toBeInTheDocument()
    );
    expect(screen.getByText('Connect')).toBeInTheDocument();
  });

  it('surfaces a connect failure as an error instead of staying stuck connecting', async () => {
    connectChannelMock.mockRejectedValue(new Error('invalid app secret'));
    renderWithProviders(<CredentialChannelConfig definition={larkDefinition} />);

    fireEvent.change(screen.getByPlaceholderText('cli_xxxxxxxxxxxx'), {
      target: { value: 'cli_abc123' },
    });
    fireEvent.change(screen.getByPlaceholderText('Your Lark app secret'), {
      target: { value: 'wrong' },
    });
    fireEvent.click(screen.getByText('Connect'));

    await waitFor(() => expect(screen.getByText('invalid app secret')).toBeInTheDocument());
    expect(restartCoreProcessMock).not.toHaveBeenCalled();
  });

  it('blocks connect and does not call the RPC when a required field is empty', async () => {
    renderWithProviders(<CredentialChannelConfig definition={larkDefinition} />);
    // Leave app_id / app_secret blank.
    fireEvent.click(screen.getByText('Connect'));
    await waitFor(() => expect(screen.getByText(/is required/i)).toBeInTheDocument());
    expect(connectChannelMock).not.toHaveBeenCalled();
  });

  it('disconnects via the api when already connected', async () => {
    disconnectChannelMock.mockResolvedValue(undefined);
    renderWithProviders(<CredentialChannelConfig definition={dingtalkDefinition} />, {
      preloadedState: {
        channelConnections: {
          schemaVersion: 1,
          migrationCompleted: true,
          defaultMessagingChannel: 'telegram',
          connections: {
            dingtalk: {
              api_key: {
                channel: 'dingtalk',
                authMode: 'api_key',
                status: 'connected',
                selectedDefault: false,
                capabilities: ['read', 'write'],
                updatedAt: new Date().toISOString(),
              },
            },
          },
        },
      },
    });

    fireEvent.click(screen.getByText('Disconnect'));
    await waitFor(() => expect(disconnectChannelMock).toHaveBeenCalledTimes(1));
    expect(disconnectChannelMock).toHaveBeenCalledWith('dingtalk', 'api_key');
  });
});
