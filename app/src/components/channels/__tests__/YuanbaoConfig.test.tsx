import { fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { channelConnectionsApi } from '../../../services/api/channelConnectionsApi';
import { setChannelConnectionStatus } from '../../../store/channelConnectionsSlice';
import { createTestStore, renderWithProviders } from '../../../test/test-utils';
import type { ChannelDefinition } from '../../../types/channels';
import { restartCoreProcess } from '../../../utils/tauriCommands/core';
import YuanbaoConfig from '../YuanbaoConfig';

vi.mock('../../../services/api/channelConnectionsApi', () => ({
  channelConnectionsApi: { connectChannel: vi.fn(), disconnectChannel: vi.fn() },
}));

vi.mock('../../../utils/tauriCommands/core', () => ({ restartCoreProcess: vi.fn() }));

// Mirrors the backend yuanbao_definition() in
// src/openhuman/channels/controllers/definitions.rs — kept inline because
// the frontend fallback definitions list does not (yet) include yuanbao.
const yuanbaoDef: ChannelDefinition = {
  id: 'yuanbao',
  display_name: '元宝',
  description: '通过元宝（Yuanbao）机器人收发消息。',
  icon: 'yuanbao',
  auth_modes: [
    {
      mode: 'api_key',
      description: '提供元宝开放平台的 AppID 和 AppSecret。',
      fields: [
        {
          key: 'app_key',
          label: 'AppID',
          field_type: 'string',
          required: true,
          placeholder: '元宝开放平台 AppID',
        },
        {
          key: 'app_secret',
          label: 'AppSecret',
          field_type: 'secret',
          required: true,
          placeholder: '元宝开放平台 AppSecret',
        },
      ],
      auth_action: undefined,
    },
  ],
  capabilities: ['send_text', 'receive_text', 'typing'],
};

afterEach(() => {
  vi.clearAllMocks();
});

describe('YuanbaoConfig', () => {
  it('renders the api_key mode label, description, and credential fields', () => {
    renderWithProviders(<YuanbaoConfig definition={yuanbaoDef} />);
    expect(screen.getByText('Use your own API Key')).toBeInTheDocument();
    expect(screen.getByText(/AppID 和 AppSecret/)).toBeInTheDocument();
    expect(screen.getByPlaceholderText('元宝开放平台 AppID')).toBeInTheDocument();
    expect(screen.getByPlaceholderText('元宝开放平台 AppSecret')).toBeInTheDocument();
  });

  it('shows a Connect and a (disabled) Disconnect button by default', () => {
    renderWithProviders(<YuanbaoConfig definition={yuanbaoDef} />);
    expect(screen.getByText('Connect')).toBeInTheDocument();
    const disconnect = screen.getByText('Disconnect');
    expect(disconnect).toBeDisabled();
  });

  it('returns null when the definition has no auth modes', () => {
    const empty: ChannelDefinition = { ...yuanbaoDef, auth_modes: [] };
    const { container } = renderWithProviders(<YuanbaoConfig definition={empty} />);
    expect(container.firstChild).toBeNull();
  });

  it('shows inline validation errors when required fields are empty and clears them on input', () => {
    renderWithProviders(<YuanbaoConfig definition={yuanbaoDef} />);
    fireEvent.click(screen.getByText('Connect'));

    // Two required fields → two inline error messages.
    const appKeyError = screen
      .getAllByText(/AppID/)
      .filter(node => node.className.includes('text-coral'));
    expect(appKeyError.length).toBeGreaterThan(0);
    expect(channelConnectionsApi.connectChannel).not.toHaveBeenCalled();

    // Typing into a field clears that field's error (covers updateField
    // branch that mutates fieldErrors).
    fireEvent.change(screen.getByPlaceholderText('元宝开放平台 AppID'), {
      target: { value: 'app-key-123' },
    });
    expect(
      screen.queryAllByText(/AppID/).filter(node => node.className.includes('text-coral')).length
    ).toBe(0);
  });

  it('connects successfully and dispatches connected when restart is not required', async () => {
    vi.mocked(channelConnectionsApi.connectChannel).mockResolvedValue({
      status: 'connected',
      restart_required: false,
    });

    const { store } = renderWithProviders(<YuanbaoConfig definition={yuanbaoDef} />);
    fireEvent.change(screen.getByPlaceholderText('元宝开放平台 AppID'), {
      target: { value: 'app-key-123' },
    });
    fireEvent.change(screen.getByPlaceholderText('元宝开放平台 AppSecret'), {
      target: { value: 'app-secret-xyz' },
    });
    fireEvent.click(screen.getByText('Connect'));

    await waitFor(() => {
      expect(channelConnectionsApi.connectChannel).toHaveBeenCalledWith('yuanbao', {
        authMode: 'api_key',
        credentials: { app_key: 'app-key-123', app_secret: 'app-secret-xyz' },
      });
    });
    await waitFor(() => {
      const conn = store.getState().channelConnections.connections.yuanbao?.api_key;
      expect(conn?.status).toBe('connected');
      expect(conn?.capabilities).toEqual(['read', 'write']);
    });
    expect(restartCoreProcess).not.toHaveBeenCalled();
  });

  it('calls restartCoreProcess and dispatches connected when restart_required=true', async () => {
    vi.mocked(channelConnectionsApi.connectChannel).mockResolvedValue({
      status: 'connected',
      restart_required: true,
    });
    vi.mocked(restartCoreProcess).mockResolvedValue();

    const { store } = renderWithProviders(<YuanbaoConfig definition={yuanbaoDef} />);
    fireEvent.change(screen.getByPlaceholderText('元宝开放平台 AppID'), {
      target: { value: 'app-key-123' },
    });
    fireEvent.change(screen.getByPlaceholderText('元宝开放平台 AppSecret'), {
      target: { value: 'app-secret-xyz' },
    });
    fireEvent.click(screen.getByText('Connect'));

    await waitFor(() => {
      expect(restartCoreProcess).toHaveBeenCalledTimes(1);
    });
    await waitFor(() => {
      const conn = store.getState().channelConnections.connections.yuanbao?.api_key;
      expect(conn?.status).toBe('connected');
    });
  });

  it('marks the channel as error when restartCoreProcess throws after a successful connect', async () => {
    vi.mocked(channelConnectionsApi.connectChannel).mockResolvedValue({
      status: 'connected',
      restart_required: true,
    });
    vi.mocked(restartCoreProcess).mockRejectedValue(new Error('core restart failed'));

    const { store } = renderWithProviders(<YuanbaoConfig definition={yuanbaoDef} />);
    fireEvent.change(screen.getByPlaceholderText('元宝开放平台 AppID'), {
      target: { value: 'app-key-123' },
    });
    fireEvent.change(screen.getByPlaceholderText('元宝开放平台 AppSecret'), {
      target: { value: 'app-secret-xyz' },
    });
    fireEvent.click(screen.getByText('Connect'));

    await waitFor(() => {
      const conn = store.getState().channelConnections.connections.yuanbao?.api_key;
      expect(conn?.status).toBe('error');
      expect(conn?.lastError).toBeTruthy();
    });
  });

  it('surfaces an error when the backend returns a non-connected status', async () => {
    vi.mocked(channelConnectionsApi.connectChannel).mockResolvedValue({
      status: 'pending_auth',
      restart_required: false,
    });

    const { store } = renderWithProviders(<YuanbaoConfig definition={yuanbaoDef} />);
    fireEvent.change(screen.getByPlaceholderText('元宝开放平台 AppID'), {
      target: { value: 'app-key-123' },
    });
    fireEvent.change(screen.getByPlaceholderText('元宝开放平台 AppSecret'), {
      target: { value: 'app-secret-xyz' },
    });
    fireEvent.click(screen.getByText('Connect'));

    await waitFor(() => {
      const conn = store.getState().channelConnections.connections.yuanbao?.api_key;
      expect(conn?.status).toBe('error');
      expect(conn?.lastError).toContain('pending_auth');
    });
  });

  it('captures connect failures from the API and dispatches an error status', async () => {
    vi.mocked(channelConnectionsApi.connectChannel).mockRejectedValue(
      new Error('invalid credentials')
    );

    const { store } = renderWithProviders(<YuanbaoConfig definition={yuanbaoDef} />);
    fireEvent.change(screen.getByPlaceholderText('元宝开放平台 AppID'), {
      target: { value: 'app-key-123' },
    });
    fireEvent.change(screen.getByPlaceholderText('元宝开放平台 AppSecret'), {
      target: { value: 'app-secret-xyz' },
    });
    fireEvent.click(screen.getByText('Connect'));

    await waitFor(() => {
      const conn = store.getState().channelConnections.connections.yuanbao?.api_key;
      expect(conn?.status).toBe('error');
      expect(conn?.lastError).toBe('invalid credentials');
    });
  });

  it('disconnects an active channel via the API and clears the connection', async () => {
    const store = createTestStore();
    store.dispatch(
      setChannelConnectionStatus({ channel: 'yuanbao', authMode: 'api_key', status: 'connected' })
    );
    vi.mocked(channelConnectionsApi.disconnectChannel).mockResolvedValue();

    renderWithProviders(<YuanbaoConfig definition={yuanbaoDef} />, { store });

    // Status is connected → Reconnect label appears on the primary button.
    expect(screen.getByText('Reconnect')).toBeInTheDocument();
    const disconnect = screen.getByText('Disconnect');
    expect(disconnect).not.toBeDisabled();
    fireEvent.click(disconnect);

    await waitFor(() => {
      expect(channelConnectionsApi.disconnectChannel).toHaveBeenCalledWith('yuanbao', 'api_key');
    });
    await waitFor(() => {
      const conn = store.getState().channelConnections.connections.yuanbao?.api_key;
      expect(conn?.status).toBe('disconnected');
    });
  });

  it('reports an error status when the disconnect API call fails', async () => {
    const store = createTestStore();
    store.dispatch(
      setChannelConnectionStatus({ channel: 'yuanbao', authMode: 'api_key', status: 'connected' })
    );
    vi.mocked(channelConnectionsApi.disconnectChannel).mockRejectedValue(
      new Error('rpc unreachable')
    );

    renderWithProviders(<YuanbaoConfig definition={yuanbaoDef} />, { store });
    fireEvent.click(screen.getByText('Disconnect'));

    await waitFor(() => {
      const conn = store.getState().channelConnections.connections.yuanbao?.api_key;
      expect(conn?.status).toBe('error');
      expect(conn?.lastError).toBe('rpc unreachable');
    });
  });

  it('resets a stale "connecting" status from a previous session on mount', () => {
    const store = createTestStore();
    store.dispatch(
      setChannelConnectionStatus({ channel: 'yuanbao', authMode: 'api_key', status: 'connecting' })
    );

    renderWithProviders(<YuanbaoConfig definition={yuanbaoDef} />, { store });

    const conn = store.getState().channelConnections.connections.yuanbao?.api_key;
    expect(conn?.status).toBe('disconnected');
  });

  it('renders the last error message when the connection is in an error state', () => {
    const store = createTestStore();
    store.dispatch(
      setChannelConnectionStatus({
        channel: 'yuanbao',
        authMode: 'api_key',
        status: 'error',
        lastError: 'sign verification failed',
      })
    );

    renderWithProviders(<YuanbaoConfig definition={yuanbaoDef} />, { store });
    expect(screen.getByText('sign verification failed')).toBeInTheDocument();
  });
});
