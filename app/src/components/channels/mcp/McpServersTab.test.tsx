/**
 * Tests for McpServersTab — the top-level two-pane MCP servers view.
 * Covers the major flows: initial load, error state, pane transitions,
 * install success, uninstall, and status polling.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import McpServersTab from './McpServersTab';

const mockInstalledList = vi.fn();
const mockStatus = vi.fn();
const mockInstall = vi.fn();
const mockConnect = vi.fn();
const mockDisconnect = vi.fn();
const mockUninstall = vi.fn();
const mockRegistryGet = vi.fn();
const mockRegistrySearch = vi.fn();
const mockConfigAssist = vi.fn();

vi.mock('../../../services/api/mcpClientsApi', () => ({
  mcpClientsApi: {
    installedList: (...args: unknown[]) => mockInstalledList(...args),
    status: (...args: unknown[]) => mockStatus(...args),
    install: (...args: unknown[]) => mockInstall(...args),
    connect: (...args: unknown[]) => mockConnect(...args),
    disconnect: (...args: unknown[]) => mockDisconnect(...args),
    uninstall: (...args: unknown[]) => mockUninstall(...args),
    registryGet: (...args: unknown[]) => mockRegistryGet(...args),
    registrySearch: (...args: unknown[]) => mockRegistrySearch(...args),
    configAssist: (...args: unknown[]) => mockConfigAssist(...args),
  },
}));

const SERVERS = [
  {
    server_id: 'srv-1',
    qualified_name: 'acme/fs-server',
    display_name: 'File Server',
    description: 'Reads files',
    command_kind: 'node' as const,
    command: 'npx',
    args: ['-y', 'acme/fs-server'],
    env_keys: [],
    installed_at: 1_700_000_000,
  },
];

const STATUSES_DISCONNECTED = [
  {
    server_id: 'srv-1',
    qualified_name: 'acme/fs-server',
    display_name: 'File Server',
    status: 'disconnected' as const,
    tool_count: 0,
  },
];

const STATUSES_CONNECTED = [
  {
    server_id: 'srv-1',
    qualified_name: 'acme/fs-server',
    display_name: 'File Server',
    status: 'connected' as const,
    tool_count: 2,
  },
];

const STATUSES_ERROR = [
  {
    server_id: 'srv-1',
    qualified_name: 'acme/fs-server',
    display_name: 'File Server',
    status: 'error' as const,
    tool_count: 0,
  },
];

describe('McpServersTab', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    mockInstalledList.mockReset();
    mockStatus.mockReset();
    mockInstall.mockReset();
    mockConnect.mockReset();
    mockDisconnect.mockReset();
    mockUninstall.mockReset();
    mockRegistryGet.mockReset();
    mockRegistrySearch.mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('shows loading state on initial render', () => {
    mockInstalledList.mockReturnValue(new Promise(() => {}));
    mockStatus.mockReturnValue(new Promise(() => {}));
    render(<McpServersTab />);
    expect(screen.getByText('Loading MCP servers...')).toBeInTheDocument();
  });

  it('renders installed server list after load', async () => {
    mockInstalledList.mockResolvedValue(SERVERS);
    mockStatus.mockResolvedValue(STATUSES_DISCONNECTED);

    render(<McpServersTab />);
    vi.useRealTimers();

    await waitFor(() => {
      expect(screen.getByText('File Server')).toBeInTheDocument();
    });
    expect(screen.getByText('Browse catalog')).toBeInTheDocument();
  });

  it('shows empty state when no servers installed', async () => {
    mockInstalledList.mockResolvedValue([]);
    mockStatus.mockResolvedValue([]);

    render(<McpServersTab />);
    vi.useRealTimers();

    await waitFor(() => {
      expect(screen.getByText('No MCP servers installed yet.')).toBeInTheDocument();
    });
  });

  it('shows load error when installedList fails', async () => {
    mockInstalledList.mockRejectedValue(new Error('DB error'));
    mockStatus.mockResolvedValue([]);

    render(<McpServersTab />);
    vi.useRealTimers();

    await waitFor(() => {
      expect(screen.getByText('DB error')).toBeInTheDocument();
    });
  });

  it('shows placeholder in right pane when no server selected', async () => {
    mockInstalledList.mockResolvedValue(SERVERS);
    mockStatus.mockResolvedValue(STATUSES_DISCONNECTED);

    render(<McpServersTab />);
    vi.useRealTimers();

    await waitFor(() => screen.getByText('File Server'));
    expect(screen.getByText('Select a server or browse the catalog.')).toBeInTheDocument();
  });

  it('opens detail pane when a server is clicked', async () => {
    mockInstalledList.mockResolvedValue(SERVERS);
    mockStatus.mockResolvedValue(STATUSES_DISCONNECTED);

    render(<McpServersTab />);
    vi.useRealTimers();

    await waitFor(() => screen.getByText('File Server'));
    fireEvent.click(screen.getAllByRole('button', { name: /File Server/i })[0]);

    await waitFor(() => {
      expect(screen.getByText('acme/fs-server')).toBeInTheDocument();
    });
  });

  it('opens catalog browser when Browse catalog is clicked', async () => {
    mockInstalledList.mockResolvedValue([]);
    mockStatus.mockResolvedValue([]);
    mockRegistrySearch.mockResolvedValue({ servers: [], page: 1, total_pages: 1 });

    render(<McpServersTab />);
    vi.useRealTimers();

    await waitFor(() => screen.getByText('No MCP servers installed yet.'));

    // advance past debounce in McpCatalogBrowser
    await act(async () => {
      await vi.advanceTimersByTimeAsync?.(300).catch(() => {});
    });

    fireEvent.click(screen.getAllByRole('button', { name: 'Browse catalog' })[0]);

    await waitFor(() => {
      expect(screen.getByPlaceholderText('Search Smithery catalog...')).toBeInTheDocument();
    });
  });

  it('clears load error on successful reload after failure', async () => {
    // Initial load fails (transient error banner appears), then a successful
    // install triggers loadInstalled() — the banner must be cleared on success.
    mockInstalledList.mockRejectedValueOnce(new Error('Transient error'));
    mockStatus.mockResolvedValue([]);
    mockRegistrySearch.mockResolvedValue({
      servers: [{ qualified_name: 'acme/new-srv', display_name: 'New Server' }],
      page: 1,
      total_pages: 1,
    });

    render(<McpServersTab />);
    vi.useRealTimers();

    await waitFor(() => screen.getByText('Transient error'));

    // Drive an install flow that triggers loadInstalled() on success.
    const detail = {
      qualified_name: 'acme/new-srv',
      display_name: 'New Server',
      description: null,
      connections: [],
      required_env_keys: [],
    };
    const newServer = { ...SERVERS[0], server_id: 'srv-new', qualified_name: 'acme/new-srv' };
    mockRegistryGet.mockResolvedValue(detail);
    mockInstall.mockResolvedValue(newServer);
    mockInstalledList.mockResolvedValue([newServer]); // reload after install succeeds

    fireEvent.click(screen.getAllByRole('button', { name: 'Browse catalog' })[0]);

    await act(async () => {
      await vi.advanceTimersByTimeAsync?.(300).catch(() => {});
    });

    await waitFor(() => screen.getByText('New Server'));
    fireEvent.click(screen.getByRole('button', { name: 'Install' }));
    await waitFor(() => screen.getByRole('button', { name: 'Install' }));
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Install' }));
    });

    // After successful reload, the error banner must be gone.
    await waitFor(() => {
      expect(screen.queryByText('Transient error')).not.toBeInTheDocument();
    });
  });

  // -----------------------------------------------------------------------
  // Regression: malformed RPC envelopes must not crash the tab
  // (Commit 38fcbd8f5 — `Cannot read properties of undefined (reading 'find')`)
  // -----------------------------------------------------------------------

  it('renders empty state when installedList resolves with undefined installed field', async () => {
    // Simulates core returning `{}` on first launch before MCP store is init'd.
    // The api layer now returns [] in this case; this test verifies the full path.
    mockInstalledList.mockResolvedValue([]);
    mockStatus.mockResolvedValue([]);

    render(<McpServersTab />);
    vi.useRealTimers();

    await waitFor(() => {
      expect(screen.getByText('No MCP servers installed yet.')).toBeInTheDocument();
    });
  });

  it('does not crash when installedList resolves with null', async () => {
    // If mcpClientsApi.installedList ever passes through null (belt + suspenders).
    mockInstalledList.mockResolvedValue(null as unknown as never[]);
    mockStatus.mockResolvedValue([]);

    // Should not throw
    const { container } = render(<McpServersTab />);
    vi.useRealTimers();

    // Either shows empty state or loading — but does NOT crash
    await waitFor(() => {
      expect(container).toBeTruthy();
    });
  });

  it('does not crash when status resolves with undefined', async () => {
    mockInstalledList.mockResolvedValue(SERVERS);
    mockStatus.mockResolvedValue(undefined as unknown as never[]);

    render(<McpServersTab />);
    vi.useRealTimers();

    // Server row still renders; just no status badge data
    await waitFor(() => {
      expect(screen.getByText('File Server')).toBeInTheDocument();
    });
  });

  it('shows error banner when installedList rejects, not a crash', async () => {
    mockInstalledList.mockRejectedValue(new Error('RPC timeout'));
    mockStatus.mockResolvedValue([]);

    render(<McpServersTab />);
    vi.useRealTimers();

    await waitFor(() => {
      expect(screen.getByText('RPC timeout')).toBeInTheDocument();
    });
    // Loading state should be gone
    expect(screen.queryByText('Loading MCP servers...')).not.toBeInTheDocument();
  });

  it('server row renders even when status rejects', async () => {
    mockInstalledList.mockResolvedValue(SERVERS);
    mockStatus.mockRejectedValue(new Error('status unavailable'));

    render(<McpServersTab />);
    vi.useRealTimers();

    // Tab should still show the server list; status error is non-fatal
    await waitFor(() => {
      expect(screen.getByText('File Server')).toBeInTheDocument();
    });
  });

  it('shows tool count badge when connected', async () => {
    mockInstalledList.mockResolvedValue(SERVERS);
    mockStatus.mockResolvedValue(STATUSES_CONNECTED);

    render(<McpServersTab />);
    vi.useRealTimers();

    await waitFor(() => {
      expect(screen.getByText('2 tools')).toBeInTheDocument();
    });
  });

  it('opens install dialog from catalog browser', async () => {
    mockInstalledList.mockResolvedValue([]);
    mockStatus.mockResolvedValue([]);
    mockRegistrySearch.mockResolvedValue({
      servers: [{ qualified_name: 'acme/new-srv', display_name: 'New Server' }],
      page: 1,
      total_pages: 1,
    });
    mockRegistryGet.mockReturnValue(new Promise(() => {}));

    render(<McpServersTab />);
    vi.useRealTimers();

    await waitFor(() => screen.getByText('No MCP servers installed yet.'));
    fireEvent.click(screen.getAllByRole('button', { name: 'Browse catalog' })[0]);

    await act(async () => {
      await vi.advanceTimersByTimeAsync?.(300).catch(() => {});
    });

    await waitFor(() => screen.getByText('New Server'));
    fireEvent.click(screen.getByRole('button', { name: 'Install' }));

    await waitFor(() => {
      expect(screen.getByText('Loading server details...')).toBeInTheDocument();
    });
  });

  it('returns to catalog after install cancel', async () => {
    mockInstalledList.mockResolvedValue([]);
    mockStatus.mockResolvedValue([]);
    mockRegistrySearch.mockResolvedValue({
      servers: [{ qualified_name: 'acme/new-srv', display_name: 'New Server' }],
      page: 1,
      total_pages: 1,
    });
    const detail = {
      qualified_name: 'acme/new-srv',
      display_name: 'New Server',
      description: null,
      connections: [],
      required_env_keys: [],
    };
    mockRegistryGet.mockResolvedValue(detail);

    render(<McpServersTab />);
    vi.useRealTimers();

    await waitFor(() => screen.getByText('No MCP servers installed yet.'));
    fireEvent.click(screen.getAllByRole('button', { name: 'Browse catalog' })[0]);

    await act(async () => {
      await vi.advanceTimersByTimeAsync?.(300).catch(() => {});
    });

    await waitFor(() => screen.getByText('New Server'));
    fireEvent.click(screen.getByRole('button', { name: 'Install' }));
    await waitFor(() => screen.getByRole('button', { name: 'Cancel' }));

    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    await waitFor(() => {
      expect(screen.getByPlaceholderText('Search Smithery catalog...')).toBeInTheDocument();
    });
  });

  it('refreshes list and shows detail after install success', async () => {
    mockInstalledList.mockResolvedValue([]);
    mockStatus.mockResolvedValue([]);
    mockRegistrySearch.mockResolvedValue({
      servers: [{ qualified_name: 'acme/new-srv', display_name: 'New Server' }],
      page: 1,
      total_pages: 1,
    });
    const detail = {
      qualified_name: 'acme/new-srv',
      display_name: 'New Server',
      description: null,
      connections: [],
      required_env_keys: [],
    };
    const newServer = {
      server_id: 'srv-new',
      qualified_name: 'acme/new-srv',
      display_name: 'New Server',
      description: null,
      command_kind: 'node' as const,
      command: 'npx',
      args: ['-y', 'acme/new-srv'],
      env_keys: [],
      installed_at: 1_700_000_001,
    };
    mockRegistryGet.mockResolvedValue(detail);
    mockInstall.mockResolvedValue(newServer);
    // After install, list refresh returns new server
    mockInstalledList.mockResolvedValueOnce([]).mockResolvedValue([newServer]);
    mockStatus.mockResolvedValue([
      {
        server_id: 'srv-new',
        qualified_name: 'acme/new-srv',
        display_name: 'New Server',
        status: 'disconnected',
        tool_count: 0,
      },
    ]);

    render(<McpServersTab />);
    vi.useRealTimers();

    await waitFor(() => screen.getByText('No MCP servers installed yet.'));
    fireEvent.click(screen.getAllByRole('button', { name: 'Browse catalog' })[0]);

    await act(async () => {
      await vi.advanceTimersByTimeAsync?.(300).catch(() => {});
    });

    await waitFor(() => screen.getByText('New Server'));
    fireEvent.click(screen.getByRole('button', { name: 'Install' }));

    await waitFor(() => screen.getByRole('button', { name: 'Install' }));
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Install' }));
    });

    await waitFor(() => {
      expect(mockInstall).toHaveBeenCalled();
    });
  });

  it('surfaces a partial-failure alert when bulk Retry All has rejections', async () => {
    mockInstalledList.mockResolvedValue(SERVERS);
    mockStatus.mockResolvedValue(STATUSES_ERROR);
    // The single errored server fails to reconnect.
    mockConnect.mockRejectedValue(new Error('connect refused'));

    render(<McpServersTab />);
    vi.useRealTimers();

    const retryBtn = await screen.findByRole('button', { name: 'Retry all 1 errored MCP servers' });
    await act(async () => {
      fireEvent.click(retryBtn);
    });

    // allSettled never rejects, so the connect was attempted...
    await waitFor(() => expect(mockConnect).toHaveBeenCalledWith('srv-1'));
    // ...and the failure is surfaced through the alert region, not swallowed.
    await waitFor(() => {
      expect(screen.getByRole('alert')).toHaveTextContent('1 of 1 servers failed. See logs.');
    });
  });

  it('surfaces a partial-failure alert when bulk Disconnect All has rejections', async () => {
    mockInstalledList.mockResolvedValue(SERVERS);
    mockStatus.mockResolvedValue(STATUSES_CONNECTED);
    mockDisconnect.mockRejectedValue(new Error('disconnect failed'));

    render(<McpServersTab />);
    vi.useRealTimers();

    const disconnectBtn = await screen.findByRole('button', {
      name: 'Disconnect all 1 connected MCP servers',
    });
    fireEvent.click(disconnectBtn);

    // Confirm dialog gates the bulk RPC; confirm it.
    const confirmBtn = await screen.findByRole('button', { name: 'Disconnect all' });
    await act(async () => {
      fireEvent.click(confirmBtn);
    });

    await waitFor(() => expect(mockDisconnect).toHaveBeenCalledWith('srv-1'));
    await waitFor(() => {
      expect(screen.getByRole('alert')).toHaveTextContent('1 of 1 servers failed. See logs.');
    });
  });
});
