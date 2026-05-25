import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import InstalledServerDetail from './InstalledServerDetail';

const mockConnect = vi.fn();
const mockDisconnect = vi.fn();
const mockUninstall = vi.fn();

vi.mock('../../../services/api/mcpClientsApi', () => ({
  mcpClientsApi: {
    connect: (...args: unknown[]) => mockConnect(...args),
    disconnect: (...args: unknown[]) => mockDisconnect(...args),
    uninstall: (...args: unknown[]) => mockUninstall(...args),
    configAssist: vi.fn(),
  },
}));

const BASE_SERVER = {
  server_id: 'srv-1',
  qualified_name: 'acme/test-server',
  display_name: 'Test Server',
  description: 'A test MCP server',
  command_kind: 'node' as const,
  command: 'node',
  args: [],
  env_keys: ['API_KEY', 'DB_URL'],
  installed_at: 1_700_000_000,
};

describe('InstalledServerDetail', () => {
  beforeEach(() => {
    mockConnect.mockReset();
    mockDisconnect.mockReset();
    mockUninstall.mockReset();
  });

  it('renders server name and description', () => {
    render(
      <InstalledServerDetail server={BASE_SERVER} connStatus={undefined} onUninstalled={() => {}} />
    );
    expect(screen.getByText('Test Server')).toBeInTheDocument();
    expect(screen.getByText('A test MCP server')).toBeInTheDocument();
  });

  it('shows env key names', () => {
    render(
      <InstalledServerDetail server={BASE_SERVER} connStatus={undefined} onUninstalled={() => {}} />
    );
    expect(screen.getByText('API_KEY')).toBeInTheDocument();
    expect(screen.getByText('DB_URL')).toBeInTheDocument();
  });

  it('shows Connect button when disconnected', () => {
    render(
      <InstalledServerDetail
        server={BASE_SERVER}
        connStatus={{
          server_id: 'srv-1',
          qualified_name: 'acme/test-server',
          display_name: 'Test Server',
          status: 'disconnected',
          tool_count: 0,
        }}
        onUninstalled={() => {}}
      />
    );
    expect(screen.getByRole('button', { name: 'Connect' })).toBeInTheDocument();
  });

  it('shows Disconnect button when connected', () => {
    render(
      <InstalledServerDetail
        server={BASE_SERVER}
        connStatus={{
          server_id: 'srv-1',
          qualified_name: 'acme/test-server',
          display_name: 'Test Server',
          status: 'connected',
          tool_count: 2,
        }}
        onUninstalled={() => {}}
      />
    );
    expect(screen.getByRole('button', { name: 'Disconnect' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Connect' })).not.toBeInTheDocument();
  });

  it('calls connect on Connect click', async () => {
    mockConnect.mockResolvedValue({ server_id: 'srv-1', status: 'connected', tools: [] });
    render(
      <InstalledServerDetail server={BASE_SERVER} connStatus={undefined} onUninstalled={() => {}} />
    );

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
    });

    expect(mockConnect).toHaveBeenCalledWith('srv-1');
  });

  it('calls disconnect on Disconnect click', async () => {
    mockDisconnect.mockResolvedValue({ server_id: 'srv-1', status: 'disconnected' });
    render(
      <InstalledServerDetail
        server={BASE_SERVER}
        connStatus={{
          server_id: 'srv-1',
          qualified_name: 'acme/test-server',
          display_name: 'Test Server',
          status: 'connected',
          tool_count: 0,
        }}
        onUninstalled={() => {}}
      />
    );

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Disconnect' }));
    });

    expect(mockDisconnect).toHaveBeenCalledWith('srv-1');
  });

  it('shows confirm prompt before uninstalling', () => {
    render(
      <InstalledServerDetail server={BASE_SERVER} connStatus={undefined} onUninstalled={() => {}} />
    );

    fireEvent.click(screen.getByRole('button', { name: 'Uninstall' }));
    expect(screen.getByRole('button', { name: 'Yes, uninstall' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeInTheDocument();
  });

  it('calls uninstall and onUninstalled after confirm', async () => {
    mockUninstall.mockResolvedValue({ server_id: 'srv-1', removed: true });
    const onUninstalled = vi.fn();
    render(
      <InstalledServerDetail
        server={BASE_SERVER}
        connStatus={undefined}
        onUninstalled={onUninstalled}
      />
    );

    fireEvent.click(screen.getByRole('button', { name: 'Uninstall' }));

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Yes, uninstall' }));
    });

    await waitFor(() => {
      expect(mockUninstall).toHaveBeenCalledWith('srv-1');
      expect(onUninstalled).toHaveBeenCalledWith('srv-1');
    });
  });

  it('shows connect error inline', async () => {
    mockConnect.mockRejectedValue(new Error('Connection refused'));
    render(
      <InstalledServerDetail server={BASE_SERVER} connStatus={undefined} onUninstalled={() => {}} />
    );

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
    });

    await waitFor(() => screen.getByText('Connection refused'));
  });

  it('renders without crashing when connStatus is undefined (no status badge data)', () => {
    // connStatus=undefined is the cold-start case before status polling resolves.
    // The component must not crash and must default to disconnected state.
    render(
      <InstalledServerDetail server={BASE_SERVER} connStatus={undefined} onUninstalled={() => {}} />
    );
    expect(screen.getByText('Test Server')).toBeInTheDocument();
    // Connect button shown (defaulted to disconnected)
    expect(screen.getByRole('button', { name: 'Connect' })).toBeInTheDocument();
    // No tool list shown in disconnected state
    expect(screen.getByText('No tools available.')).toBeInTheDocument();
  });

  it('renders status badge from connStatus', () => {
    render(
      <InstalledServerDetail
        server={BASE_SERVER}
        connStatus={{
          server_id: 'srv-1',
          qualified_name: 'acme/test-server',
          display_name: 'Test Server',
          status: 'error',
          tool_count: 0,
          last_error: 'Timed out',
        }}
        onUninstalled={() => {}}
      />
    );
    expect(screen.getByText('Error')).toBeInTheDocument();
    // last_error shown in the error banner
    expect(screen.getByText('Timed out')).toBeInTheDocument();
  });
});
