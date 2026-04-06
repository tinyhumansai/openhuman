import { screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import DiscordServerChannelPicker from '../DiscordServerChannelPicker';

// Mock the RPC client to avoid actual network calls
vi.mock('../../../services/coreRpcClient', () => ({
  callCoreRpc: vi.fn().mockImplementation(({ method }: { method: string }) => {
    if (method === 'openhuman.channels_discord_list_guilds') {
      return Promise.resolve([
        { id: '111', name: 'Test Server', icon: null },
        { id: '222', name: 'Another Server', icon: 'abc' },
      ]);
    }
    if (method === 'openhuman.channels_discord_list_channels') {
      return Promise.resolve([
        { id: '901', name: 'general', type: 0, position: 0, parent_id: null },
        { id: '902', name: 'dev', type: 0, position: 1, parent_id: '800' },
      ]);
    }
    if (method === 'openhuman.channels_discord_check_permissions') {
      return Promise.resolve({
        can_view_channel: true,
        can_send_messages: true,
        can_read_message_history: true,
        missing_permissions: [],
      });
    }
    return Promise.reject(new Error(`unexpected RPC method: ${method}`));
  }),
}));

describe('DiscordServerChannelPicker', () => {
  it('renders server selection heading', () => {
    renderWithProviders(<DiscordServerChannelPicker />);
    expect(screen.getByText('Server & Channel Selection')).toBeInTheDocument();
  });

  it('loads and displays guilds', async () => {
    renderWithProviders(<DiscordServerChannelPicker />);
    await waitFor(() => {
      expect(screen.getByText('Test Server')).toBeInTheDocument();
      expect(screen.getByText('Another Server')).toBeInTheDocument();
    });
  });

  it('shows "Select a server" placeholder after guilds load', async () => {
    renderWithProviders(<DiscordServerChannelPicker />);
    await waitFor(() => {
      expect(screen.getByRole('combobox', { name: /server/i })).toBeInTheDocument();
    });
  });
});
