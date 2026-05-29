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

  // ----------------------------------------------------------------------
  // Tool Execution Playground gating (PR review fix)
  // ----------------------------------------------------------------------

  /**
   * Disconnected → connect → connected re-render flow. Returns the
   * rerender function so the caller can flip status further. By the
   * time this resolves the playground modal is open against the
   * `read_file` tool from the mocked connect result.
   */
  const setupOpenPlayground = async () => {
    mockConnect.mockResolvedValue({
      server_id: 'srv-1',
      status: 'connected',
      tools: [{ name: 'read_file', description: 'reads', input_schema: {} }],
    });
    const disconnectedStatus = {
      server_id: 'srv-1',
      qualified_name: 'acme/test-server',
      display_name: 'Test Server',
      status: 'disconnected' as const,
      tool_count: 0,
    };
    const connectedStatus = { ...disconnectedStatus, status: 'connected' as const, tool_count: 1 };
    const { rerender } = render(
      <InstalledServerDetail
        server={BASE_SERVER}
        connStatus={disconnectedStatus}
        onUninstalled={() => {}}
      />
    );
    // Click Connect — fills the local `tools` state via the mocked RPC.
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
    });
    // Parent would now flip status to connected (driven by its poll
    // loop after install/connect succeeds); simulate that here.
    rerender(
      <InstalledServerDetail
        server={BASE_SERVER}
        connStatus={connectedStatus}
        onUninstalled={() => {}}
      />
    );
    // Expand the tool list to reveal the Try button, then click Try.
    fireEvent.click(screen.getByRole('button', { name: /tool available/i }));
    fireEvent.click(
      screen.getByRole('button', { name: 'Open execution playground for read_file' })
    );
    expect(screen.getByRole('dialog')).toBeInTheDocument();
    return { rerender, connectedStatus };
  };

  it('clears the playground when Disconnect is clicked (handler path)', async () => {
    mockDisconnect.mockResolvedValue({ server_id: 'srv-1', status: 'disconnected' });
    await setupOpenPlayground();
    // Click Disconnect — handler calls setPlaygroundTool(null).
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Disconnect' }));
    });
    await waitFor(() => {
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    });
  });

  it('hides the playground via the render gate when status flips externally', async () => {
    const { rerender, connectedStatus } = await setupOpenPlayground();
    // External status flip (e.g. driven by the parent's poll loop).
    // The gate `status === "connected"` must hide the modal even
    // though no handler ran inside the detail component.
    rerender(
      <InstalledServerDetail
        server={BASE_SERVER}
        connStatus={{ ...connectedStatus, status: 'error', last_error: 'boom' }}
        onUninstalled={() => {}}
      />
    );
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('clears playground STATE on external status flip, so it does not reappear on reconnect', async () => {
    const { rerender, connectedStatus } = await setupOpenPlayground();
    // Poll-driven flip away from connected. The render gate hides the modal,
    // and the status-watching effect must additionally clear playgroundTool.
    rerender(
      <InstalledServerDetail
        server={BASE_SERVER}
        connStatus={{ ...connectedStatus, status: 'error', last_error: 'boom' }}
        onUninstalled={() => {}}
      />
    );
    await waitFor(() => {
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    });
    // Reconnect. If the effect only relied on the render gate (state still
    // set), the modal would spring back open here. With the state cleared it
    // must stay closed until the user explicitly clicks Try again.
    rerender(
      <InstalledServerDetail
        server={BASE_SERVER}
        connStatus={connectedStatus}
        onUninstalled={() => {}}
      />
    );
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });
});
