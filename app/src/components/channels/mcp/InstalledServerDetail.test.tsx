import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import InstalledServerDetail from './InstalledServerDetail';

const mockConnect = vi.fn();
const mockDisconnect = vi.fn();
const mockUninstall = vi.fn();
const mockUpdateEnv = vi.fn();
const mockDetectAuth = vi.fn();
const mockRegistryGet = vi.fn();

vi.mock('../../../services/api/mcpClientsApi', () => ({
  mcpClientsApi: {
    connect: (...args: unknown[]) => mockConnect(...args),
    disconnect: (...args: unknown[]) => mockDisconnect(...args),
    uninstall: (...args: unknown[]) => mockUninstall(...args),
    updateEnv: (...args: unknown[]) => mockUpdateEnv(...args),
    detectAuth: (...args: unknown[]) => mockDetectAuth(...args),
    registryGet: (...args: unknown[]) => mockRegistryGet(...args),
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
  enabled: true,
};

describe('InstalledServerDetail', () => {
  beforeEach(() => {
    mockConnect.mockReset();
    mockDisconnect.mockReset();
    mockUninstall.mockReset();
    mockUpdateEnv.mockReset();
    mockDetectAuth.mockReset();
    mockRegistryGet.mockReset();
    // The ConnectAuthModal probes auth on open (detectAuth) and best-effort
    // pulls declared keys (registryGet). Default both to benign resolutions so
    // the modal renders its token/header fields rather than hanging in
    // `detecting` or throwing. Individual tests can override.
    mockDetectAuth.mockResolvedValue({ kind: 'none', grant_types: [] });
    mockRegistryGet.mockResolvedValue({
      qualified_name: 'acme/test-server',
      display_name: 'Test Server',
      connections: [],
      required_env_keys: [],
    });
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

  it('shows Connecting… label and disables the Connect button while status=connecting', () => {
    render(
      <InstalledServerDetail
        server={BASE_SERVER}
        connStatus={{
          server_id: 'srv-1',
          qualified_name: 'acme/test-server',
          display_name: 'Test Server',
          status: 'connecting',
          tool_count: 0,
        }}
        onUninstalled={() => {}}
      />
    );
    const btn = screen.getByRole('button', { name: /^connecting/i });
    expect(btn).toBeInTheDocument();
    expect(btn).toBeDisabled();
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

  it('calls connect on Connect click (routed through the auth modal)', async () => {
    // Connect no longer dials directly — it opens ConnectAuthModal, which
    // performs the authenticated connect. Drive that flow and assert the
    // modal's connect submit reaches mcpClientsApi.connect (no auth supplied,
    // so it takes the plain `connect` path rather than update_env).
    mockConnect.mockResolvedValue({ server_id: 'srv-1', status: 'connected', tools: [] });
    render(
      <InstalledServerDetail server={BASE_SERVER} connStatus={undefined} onUninstalled={() => {}} />
    );

    // Click the page's Connect button → opens the modal.
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
    });
    const dialog = await screen.findByRole('dialog');

    // Submit the modal's own Connect button (the in-dialog action).
    await act(async () => {
      fireEvent.click(within(dialog).getByRole('button', { name: 'Connect' }));
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

  it('surfaces a failed connect as an error in the auth modal', async () => {
    // A failed connect now surfaces inside ConnectAuthModal (the modal owns the
    // connect call), not on the detail page. Drive the modal flow and assert the
    // error message appears within the dialog.
    mockConnect.mockRejectedValue(new Error('Connection refused'));
    render(
      <InstalledServerDetail server={BASE_SERVER} connStatus={undefined} onUninstalled={() => {}} />
    );

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
    });
    const dialog = await screen.findByRole('dialog');

    await act(async () => {
      fireEvent.click(within(dialog).getByRole('button', { name: 'Connect' }));
    });

    await waitFor(() => {
      expect(within(dialog).getByText('Connection refused')).toBeInTheDocument();
    });
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

  it('renders a graceful auth notice (not a raw error) for unauthorized status', () => {
    render(
      <InstalledServerDetail
        server={BASE_SERVER}
        connStatus={{
          server_id: 'srv-1',
          qualified_name: 'acme/test-server',
          display_name: 'Test Server',
          status: 'unauthorized',
          tool_count: 0,
          // Core sends no raw error for a 401 (avoids leaking the OAuth URL).
          last_error: undefined,
        }}
        onUninstalled={() => {}}
      />
    );
    // Friendly "sign in needed" badge + actionable notice, no raw HTTP string.
    expect(screen.getByText('Sign in needed')).toBeInTheDocument();
    expect(screen.getByText(/needs you to sign in or add an access token/i)).toBeInTheDocument();
    expect(screen.queryByText(/HTTP 401/i)).not.toBeInTheDocument();
    // The primary action is relabelled "Sign in" (it opens the auth modal).
    expect(screen.getByRole('button', { name: 'Sign in' })).toBeInTheDocument();
  });

  // ----------------------------------------------------------------------
  // Env reconfiguration (issue #3039)
  // ----------------------------------------------------------------------

  it('opens the reconfigure form with one input per env key', () => {
    render(
      <InstalledServerDetail server={BASE_SERVER} connStatus={undefined} onUninstalled={() => {}} />
    );
    fireEvent.click(screen.getByRole('button', { name: 'Reconfigure' }));
    expect(screen.getByLabelText('API_KEY')).toBeInTheDocument();
    expect(screen.getByLabelText('DB_URL')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Save & reconnect' })).toBeInTheDocument();
  });

  it('validates that every env key is filled before saving', async () => {
    render(
      <InstalledServerDetail server={BASE_SERVER} connStatus={undefined} onUninstalled={() => {}} />
    );
    fireEvent.click(screen.getByRole('button', { name: 'Reconfigure' }));
    // Fill only one of the two keys.
    fireEvent.change(screen.getByLabelText('API_KEY'), { target: { value: 'k' } });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Save & reconnect' }));
    });
    expect(screen.getByText('"DB_URL" is required')).toBeInTheDocument();
    expect(mockUpdateEnv).not.toHaveBeenCalled();
  });

  it('calls updateEnv with all values and shows success on reconnect', async () => {
    mockUpdateEnv.mockResolvedValue({
      server_id: 'srv-1',
      status: 'connected',
      env_keys: ['API_KEY', 'DB_URL'],
      tools: [],
    });
    render(
      <InstalledServerDetail server={BASE_SERVER} connStatus={undefined} onUninstalled={() => {}} />
    );
    fireEvent.click(screen.getByRole('button', { name: 'Reconfigure' }));
    fireEvent.change(screen.getByLabelText('API_KEY'), { target: { value: 'new-key' } });
    fireEvent.change(screen.getByLabelText('DB_URL'), { target: { value: 'new-url' } });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Save & reconnect' }));
    });
    expect(mockUpdateEnv).toHaveBeenCalledWith({
      server_id: 'srv-1',
      env: { API_KEY: 'new-key', DB_URL: 'new-url' },
    });
    await waitFor(() =>
      expect(screen.getByText('Environment updated and reconnected.')).toBeInTheDocument()
    );
  });

  it('surfaces an error when reconnect after update fails', async () => {
    mockUpdateEnv.mockResolvedValue({
      server_id: 'srv-1',
      status: 'disconnected',
      env_keys: ['API_KEY', 'DB_URL'],
      error: 'bad token',
    });
    render(
      <InstalledServerDetail server={BASE_SERVER} connStatus={undefined} onUninstalled={() => {}} />
    );
    fireEvent.click(screen.getByRole('button', { name: 'Reconfigure' }));
    fireEvent.change(screen.getByLabelText('API_KEY'), { target: { value: 'k' } });
    fireEvent.change(screen.getByLabelText('DB_URL'), { target: { value: 'u' } });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Save & reconnect' }));
    });
    await waitFor(() => expect(screen.getByText('bad token')).toBeInTheDocument());
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
    // Connect now opens ConnectAuthModal; the modal performs the actual connect
    // and calls back onConnected(tools), which fills the local `tools` state.
    // Drive that two-step flow: open the modal, then submit its Connect button.
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
    });
    const dialog = await screen.findByRole('dialog');
    await act(async () => {
      fireEvent.click(within(dialog).getByRole('button', { name: 'Connect' }));
    });
    // Modal closes itself on success (onConnected → setConnectModalOpen(false)).
    await waitFor(() => {
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
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
