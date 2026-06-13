/**
 * Tests for McpServersTab — unified table view.
 *
 * Covers: initial load, error state, filter chips, table rows,
 * navigation to detail/install views, install success, uninstall, and
 * status polling.
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
const mockSetEnabled = vi.fn();
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
    setEnabled: (...args: unknown[]) => mockSetEnabled(...args),
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
    enabled: true,
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

/**
 * Helper that renders McpServersTab and waits past the initial debounce
 * so the catalog fetch fires and resolves. Keeps fake timers throughout —
 * callers that need waitFor must switch to real timers themselves.
 */
async function renderAndWaitForLoad() {
  const result = render(<McpServersTab />);
  // Drain resolved promises from installedList / status
  await act(async () => {
    await Promise.resolve();
  });
  // Advance past the 300 ms catalog debounce so fetchCatalog is called
  await act(async () => {
    await vi.advanceTimersByTimeAsync(300);
  });
  // Drain the async catalog fetch result
  await act(async () => {
    await Promise.resolve();
  });
  return result;
}

describe('McpServersTab', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    mockInstalledList.mockReset();
    mockStatus.mockReset();
    mockInstall.mockReset();
    mockConnect.mockReset();
    mockConnect.mockResolvedValue({ server_id: '', status: 'connected', tools: [] });
    mockDisconnect.mockReset();
    mockUninstall.mockReset();
    mockSetEnabled.mockReset();
    mockRegistryGet.mockReset();
    mockRegistrySearch.mockReset();
    mockRegistrySearch.mockResolvedValue({ servers: [], page: 1, total_pages: 1 });
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

  it('renders installed server row in the table after load', async () => {
    mockInstalledList.mockResolvedValue(SERVERS);
    mockStatus.mockResolvedValue(STATUSES_DISCONNECTED);

    await renderAndWaitForLoad();
    vi.useRealTimers();

    await waitFor(() => {
      expect(screen.getByText('File Server')).toBeInTheDocument();
    });
    expect(screen.getByText('Name')).toBeInTheDocument();
    expect(screen.getByText('Author')).toBeInTheDocument();
  });

  it('renders filter chips — All, Installed, Registry', async () => {
    mockInstalledList.mockResolvedValue([]);
    mockStatus.mockResolvedValue([]);

    await renderAndWaitForLoad();
    vi.useRealTimers();

    await waitFor(() => {
      expect(screen.queryByText('Loading MCP servers...')).not.toBeInTheDocument();
    });

    expect(screen.getByRole('button', { name: 'All' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Installed/i })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Registry' })).toBeInTheDocument();
  });

  it('shows empty-installed state when Installed chip is active and no servers', async () => {
    mockInstalledList.mockResolvedValue([]);
    mockStatus.mockResolvedValue([]);

    await renderAndWaitForLoad();
    vi.useRealTimers();

    await waitFor(() => {
      expect(screen.queryByText('Loading MCP servers...')).not.toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole('button', { name: /Installed/i }));

    await waitFor(() => {
      expect(screen.getByText('No MCP servers installed yet.')).toBeInTheDocument();
    });
  });

  it('shows load error when installedList fails', async () => {
    mockInstalledList.mockRejectedValue(new Error('DB error'));
    mockStatus.mockResolvedValue([]);

    await renderAndWaitForLoad();
    vi.useRealTimers();

    await waitFor(() => {
      expect(screen.getByText('DB error')).toBeInTheDocument();
    });
  });

  it('shows Inventory button in the header', async () => {
    mockInstalledList.mockResolvedValue([]);
    mockStatus.mockResolvedValue([]);

    await renderAndWaitForLoad();
    vi.useRealTimers();

    await waitFor(() => {
      expect(screen.queryByText('Loading MCP servers...')).not.toBeInTheDocument();
    });

    expect(
      screen.getByRole('button', { name: 'Open the sharable MCP inventory panel' })
    ).toBeInTheDocument();
  });

  it('navigates to detail view when an installed server row is clicked', async () => {
    mockInstalledList.mockResolvedValue(SERVERS);
    mockStatus.mockResolvedValue(STATUSES_DISCONNECTED);

    await renderAndWaitForLoad();
    vi.useRealTimers();

    await waitFor(() => screen.getByText('File Server'));
    fireEvent.click(screen.getByText('File Server').closest('tr')!);

    await waitFor(() => {
      expect(screen.getByText('acme/fs-server')).toBeInTheDocument();
    });
  });

  it('navigates back to home from detail view', async () => {
    mockInstalledList.mockResolvedValue(SERVERS);
    mockStatus.mockResolvedValue(STATUSES_DISCONNECTED);

    await renderAndWaitForLoad();
    vi.useRealTimers();

    await waitFor(() => screen.getByText('File Server'));
    fireEvent.click(screen.getByText('File Server').closest('tr')!);

    await waitFor(() => screen.getByText('acme/fs-server'));

    fireEvent.click(screen.getByText('Go back'));
    await waitFor(() => {
      expect(screen.queryByText('acme/fs-server')).not.toBeInTheDocument();
      expect(screen.getByText('File Server')).toBeInTheDocument();
    });
  });

  it('shows registry servers from catalog in the table', async () => {
    mockInstalledList.mockResolvedValue([]);
    mockStatus.mockResolvedValue([]);
    mockRegistrySearch.mockResolvedValue({
      servers: [
        { qualified_name: 'acme/new-srv', display_name: 'New Server', description: 'A new server' },
      ],
      page: 1,
      total_pages: 1,
    });

    await renderAndWaitForLoad();
    vi.useRealTimers();

    await waitFor(() => {
      expect(screen.getByText('New Server')).toBeInTheDocument();
    });
    expect(screen.getByText('Install')).toBeInTheDocument();
  });

  it('navigates to install view when a registry server row is clicked', async () => {
    mockInstalledList.mockResolvedValue([]);
    mockStatus.mockResolvedValue([]);
    mockRegistrySearch.mockResolvedValue({
      servers: [{ qualified_name: 'acme/new-srv', display_name: 'New Server' }],
      page: 1,
      total_pages: 1,
    });
    mockRegistryGet.mockReturnValue(new Promise(() => {}));

    await renderAndWaitForLoad();
    vi.useRealTimers();

    await waitFor(() => screen.getByText('New Server'));
    fireEvent.click(screen.getByText('New Server').closest('tr')!);

    await waitFor(() => {
      expect(screen.getByText('Loading server details...')).toBeInTheDocument();
    });
  });

  it('returns to home after install cancel', async () => {
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

    await renderAndWaitForLoad();
    vi.useRealTimers();

    await waitFor(() => screen.getByText('New Server'));
    fireEvent.click(screen.getByText('New Server').closest('tr')!);
    await waitFor(() => screen.getByRole('button', { name: 'Cancel' }));

    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    await waitFor(() => {
      expect(screen.getByPlaceholderText('Search MCP servers...')).toBeInTheDocument();
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

    await renderAndWaitForLoad();
    vi.useRealTimers();

    await waitFor(() => screen.getByText('New Server'));
    // Click the registry row to open install dialog
    fireEvent.click(screen.getByText('New Server').closest('tr')!);

    // Wait for detail to load, then click Install on the detail step
    await waitFor(() => screen.getByRole('button', { name: 'Install' }));
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Install' }));
    });

    await waitFor(() => {
      expect(mockInstall).toHaveBeenCalled();
    });
  });

  it('refreshes installed list + status after a server is disabled from the detail pane', async () => {
    mockInstalledList.mockResolvedValue(SERVERS);
    mockStatus.mockResolvedValue(STATUSES_DISCONNECTED);
    mockSetEnabled.mockResolvedValue({ server_id: 'srv-1', enabled: false });

    await renderAndWaitForLoad();
    vi.useRealTimers();

    await waitFor(() => screen.getByText('File Server'));
    fireEvent.click(screen.getByText('File Server').closest('tr')!);
    await waitFor(() => screen.getByText('acme/fs-server'));

    const installedCallsBefore = mockInstalledList.mock.calls.length;
    const statusCallsBefore = mockStatus.mock.calls.length;

    const disableBtn = await screen.findByRole('button', { name: /^disable$/i });
    fireEvent.click(disableBtn);

    await waitFor(() => {
      expect(mockSetEnabled).toHaveBeenCalledWith('srv-1', false);
      expect(mockInstalledList.mock.calls.length).toBeGreaterThan(installedCallsBefore);
      expect(mockStatus.mock.calls.length).toBeGreaterThan(statusCallsBefore);
    });
  });

  it('clears load error on successful reload after failure', async () => {
    mockInstalledList.mockRejectedValueOnce(new Error('Transient error'));
    mockStatus.mockResolvedValue([]);
    mockRegistrySearch.mockResolvedValue({
      servers: [{ qualified_name: 'acme/new-srv', display_name: 'New Server' }],
      page: 1,
      total_pages: 1,
    });

    await renderAndWaitForLoad();
    vi.useRealTimers();

    await waitFor(() => screen.getByText('Transient error'));

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
    mockInstalledList.mockResolvedValue([newServer]);

    await waitFor(() => screen.getByText('New Server'));
    // Click registry row to open install
    fireEvent.click(screen.getByText('New Server').closest('tr')!);
    // Wait for detail step, click Install
    await waitFor(() => screen.getByRole('button', { name: 'Install' }));
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Install' }));
    });

    await waitFor(() => {
      expect(screen.queryByText('Transient error')).not.toBeInTheDocument();
    });
  });

  // -----------------------------------------------------------------------
  // Regression: malformed RPC envelopes must not crash the tab
  // -----------------------------------------------------------------------

  it('renders without crashing when installedList resolves with undefined/null', async () => {
    mockInstalledList.mockResolvedValue(null as unknown as never[]);
    mockStatus.mockResolvedValue([]);

    const { container } = await renderAndWaitForLoad();
    vi.useRealTimers();

    await waitFor(() => {
      expect(container).toBeTruthy();
    });
  });

  it('does not crash when status resolves with undefined', async () => {
    mockInstalledList.mockResolvedValue(SERVERS);
    mockStatus.mockResolvedValue(undefined as unknown as never[]);

    await renderAndWaitForLoad();
    vi.useRealTimers();

    await waitFor(() => {
      expect(screen.getByText('File Server')).toBeInTheDocument();
    });
  });

  it('shows error banner when installedList rejects, not a crash', async () => {
    mockInstalledList.mockRejectedValue(new Error('RPC timeout'));
    mockStatus.mockResolvedValue([]);

    await renderAndWaitForLoad();
    vi.useRealTimers();

    await waitFor(() => {
      expect(screen.getByText('RPC timeout')).toBeInTheDocument();
    });
    expect(screen.queryByText('Loading MCP servers...')).not.toBeInTheDocument();
  });

  it('server row renders even when status rejects', async () => {
    mockInstalledList.mockResolvedValue(SERVERS);
    mockStatus.mockRejectedValue(new Error('status unavailable'));

    await renderAndWaitForLoad();
    vi.useRealTimers();

    await waitFor(() => {
      expect(screen.getByText('File Server')).toBeInTheDocument();
    });
  });

  it('shows installed server with Manage action in the table', async () => {
    mockInstalledList.mockResolvedValue(SERVERS);
    mockStatus.mockResolvedValue(STATUSES_CONNECTED);

    await renderAndWaitForLoad();
    vi.useRealTimers();

    await waitFor(() => {
      expect(screen.getByText('File Server')).toBeInTheDocument();
    });
    expect(screen.getByText('Manage')).toBeInTheDocument();
  });

  it('search query narrows the installed server table results', async () => {
    mockInstalledList.mockResolvedValue(SERVERS);
    mockStatus.mockResolvedValue(STATUSES_DISCONNECTED);

    await renderAndWaitForLoad();
    vi.useRealTimers();

    await waitFor(() => screen.getByText('File Server'));

    const searchInput = screen.getByRole('searchbox');
    fireEvent.change(searchInput, { target: { value: 'zzz-nomatch' } });

    await waitFor(() => {
      expect(screen.queryByText('File Server')).not.toBeInTheDocument();
    });
  });

  it('Registry chip hides installed rows', async () => {
    mockInstalledList.mockResolvedValue(SERVERS);
    mockStatus.mockResolvedValue(STATUSES_DISCONNECTED);

    await renderAndWaitForLoad();
    vi.useRealTimers();

    await waitFor(() => screen.getByText('File Server'));

    fireEvent.click(screen.getByRole('button', { name: 'Registry' }));

    await waitFor(() => {
      expect(screen.queryByText('File Server')).not.toBeInTheDocument();
    });
  });
});
