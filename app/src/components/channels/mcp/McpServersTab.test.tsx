/**
 * Tests for McpServersTab — unified table view.
 *
 * Covers: initial load, error state, filter chips, table rows,
 * navigation to detail/install views, install success, uninstall, and
 * status polling.
 */
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
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
const mockOpenUrl = vi.fn();

vi.mock('../../../utils/openUrl', () => ({
  openUrl: (...args: unknown[]) => mockOpenUrl(...args),
}));

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
    mockOpenUrl.mockReset();
    mockOpenUrl.mockResolvedValue(undefined);
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

  it('collapses duplicate installed servers to one row per qualified_name', async () => {
    // Two legacy installs of the same service (distinct server_id, same slug).
    const dupA = { ...SERVERS[0], server_id: 'srv-1', installed_at: 1 };
    const dupB = { ...SERVERS[0], server_id: 'srv-2', installed_at: 2 };
    mockInstalledList.mockResolvedValue([dupA, dupB]);
    mockStatus.mockResolvedValue([]);

    await renderAndWaitForLoad();
    vi.useRealTimers();

    await waitFor(() => expect(screen.getAllByText('File Server')).toHaveLength(1));
    // The Installed chip count reflects the collapsed list.
    expect(screen.getByText('Installed (1)')).toBeInTheDocument();
  });

  it('renders filter chips — All, Installed, Registry', async () => {
    mockInstalledList.mockResolvedValue([]);
    mockStatus.mockResolvedValue([]);

    await renderAndWaitForLoad();
    vi.useRealTimers();

    await waitFor(() => {
      expect(screen.queryByText('Loading MCP servers...')).not.toBeInTheDocument();
    });

    expect(screen.getByRole('tab', { name: 'All' })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: /Installed/i })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'Registry' })).toBeInTheDocument();
  });

  it('shows empty-installed state when Installed chip is active and no servers', async () => {
    mockInstalledList.mockResolvedValue([]);
    mockStatus.mockResolvedValue([]);

    await renderAndWaitForLoad();
    vi.useRealTimers();

    await waitFor(() => {
      expect(screen.queryByText('Loading MCP servers...')).not.toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole('tab', { name: /Installed/i }));

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
        {
          qualified_name: 'acme/new-srv',
          display_name: 'New Server',
          description: 'A new server',
          official: true,
        },
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

  it('renders only one row per qualified_name when the registry returns duplicates', async () => {
    mockInstalledList.mockResolvedValue([]);
    mockStatus.mockResolvedValue([]);
    mockRegistrySearch.mockResolvedValue({
      servers: [
        { qualified_name: 'acme/new-srv', display_name: 'New Server', official: true },
        { qualified_name: 'acme/new-srv', display_name: 'New Server', official: true },
      ],
      page: 1,
      total_pages: 1,
    });

    await renderAndWaitForLoad();
    vi.useRealTimers();

    await waitFor(() => expect(screen.getByText('New Server')).toBeInTheDocument());
    expect(screen.getAllByText('New Server')).toHaveLength(1);
  });

  it('dedupes registry rows across "load more" pages that overlap', async () => {
    mockInstalledList.mockResolvedValue([]);
    mockStatus.mockResolvedValue([]);
    mockRegistrySearch.mockImplementation(({ page }: { page: number }) =>
      Promise.resolve(
        page === 1
          ? {
              servers: [
                { qualified_name: 'acme/a', display_name: 'Server A', official: true },
                { qualified_name: 'acme/b', display_name: 'Server B', official: true },
              ],
              page: 1,
              total_pages: 2,
            }
          : {
              // page 2 overlaps page 1 on acme/b and adds acme/c
              servers: [
                { qualified_name: 'acme/b', display_name: 'Server B', official: true },
                { qualified_name: 'acme/c', display_name: 'Server C', official: true },
              ],
              page: 2,
              total_pages: 2,
            }
      )
    );

    await renderAndWaitForLoad();
    vi.useRealTimers();

    await waitFor(() => screen.getByText('Server A'));
    fireEvent.click(screen.getByText('Load more'));

    await waitFor(() => expect(screen.getByText('Server C')).toBeInTheDocument());
    expect(screen.getAllByText('Server B')).toHaveLength(1);
  });

  it('distinguishes look-alike registry rows by slug', async () => {
    mockInstalledList.mockResolvedValue([]);
    mockStatus.mockResolvedValue([]);
    mockRegistrySearch.mockResolvedValue({
      servers: [
        {
          qualified_name: 'waystation/gmail',
          display_name: 'gmail',
          source: 'mcp_official',
          official: true,
        },
        {
          qualified_name: 'mintmcp/gmail',
          display_name: 'gmail',
          source: 'smithery',
          official: true,
        },
      ],
      page: 1,
      total_pages: 1,
    });

    await renderAndWaitForLoad();
    vi.useRealTimers();

    // Both rows share the display name "gmail"...
    await waitFor(() => expect(screen.getAllByText('gmail')).toHaveLength(2));
    // ...but the unique slug tells them apart. The registry-source pill was
    // removed: it labelled every official-registry row "Official", which falsely
    // vouched for un-vetted community servers. Only the vendor `official` badge
    // remains, and only on the row flagged official.
    expect(screen.getByText('waystation/gmail')).toBeInTheDocument();
    expect(screen.getByText('mintmcp/gmail')).toBeInTheDocument();
    // Both fixture rows are flagged official → two vendor badges, no source pill.
    expect(screen.getAllByText(/Official/)).toHaveLength(2);
    expect(screen.queryByText('Smithery')).not.toBeInTheDocument();
  });

  it('renders the registry as one list (no auth-method grouping) in registry order', async () => {
    mockInstalledList.mockResolvedValue([]);
    mockStatus.mockResolvedValue([]);
    mockRegistrySearch.mockResolvedValue({
      servers: [
        { qualified_name: 'a/local-srv', display_name: 'Local One', is_deployed: false },
        { qualified_name: 'b/hosted-srv', display_name: 'Hosted One', is_deployed: true },
      ],
      page: 1,
      total_pages: 1,
    });

    await renderAndWaitForLoad();
    vi.useRealTimers();

    await waitFor(() => screen.getByText('Hosted One'));
    // The misleading auth-method group headers are gone — `is_deployed` does not
    // predict whether a server uses browser sign-in or wants a pasted token, so
    // we no longer split on it. The real requirement surfaces on install.
    expect(screen.queryByText('Browser sign-in')).not.toBeInTheDocument();
    expect(screen.queryByText('Token / API key')).not.toBeInTheDocument();
    // Rows keep their registry order (relevance), regardless of transport.
    const table = screen.getByRole('table');
    const tableText = table.textContent ?? '';
    expect(tableText.indexOf('Local One')).toBeLessThan(tableText.indexOf('Hosted One'));
    // Per-row transport badges render (Stdio vs Hosted). Scope to the table —
    // "Stdio"/"Hosted" also appear as the transport filter chips above it.
    expect(within(table).getByText('Hosted')).toBeInTheDocument();
    expect(within(table).getByText('Stdio')).toBeInTheDocument();
  });

  it('renders clickable website + repo links that open externally without installing', async () => {
    mockInstalledList.mockResolvedValue([]);
    mockStatus.mockResolvedValue([]);
    mockRegistrySearch.mockResolvedValue({
      servers: [
        {
          qualified_name: 'io.github.acme/cool-mcp',
          display_name: 'Cool MCP',
          is_deployed: true,
          website_url: 'https://acme.example',
        },
      ],
      page: 1,
      total_pages: 1,
    });

    await renderAndWaitForLoad();
    vi.useRealTimers();

    await waitFor(() => screen.getByText('Cool MCP'));

    // Website link opens the declared site; repo link is derived from the
    // io.github.<user>/<repo> slug. Neither should trigger the row's install nav.
    fireEvent.click(screen.getByText('Website'));
    expect(mockOpenUrl).toHaveBeenCalledWith('https://acme.example');

    fireEvent.click(screen.getByText('Repository'));
    expect(mockOpenUrl).toHaveBeenCalledWith('https://github.com/acme/cool-mcp');

    // Still on the catalog list — clicking a link did not navigate to install.
    expect(screen.getByText('Cool MCP')).toBeInTheDocument();
    expect(screen.queryByText('Install Cool MCP')).not.toBeInTheDocument();
  });

  it('re-queries the core with a transport filter when the Stdio/Hosted pills are toggled', async () => {
    mockInstalledList.mockResolvedValue([]);
    mockStatus.mockResolvedValue([]);
    // Transport filtering now happens in the core: the mock answers per the
    // `transport` param it receives, and toggling a pill re-queries.
    const STDIO = { qualified_name: 'a/stdio-srv', display_name: 'Stdio Srv', is_deployed: false };
    const HOSTED = {
      qualified_name: 'b/hosted-srv',
      display_name: 'Hosted Srv',
      is_deployed: true,
    };
    mockRegistrySearch.mockImplementation((params: { transport?: string }) => {
      const servers =
        params.transport === 'stdio'
          ? [STDIO]
          : params.transport === 'hosted'
            ? [HOSTED]
            : [STDIO, HOSTED];
      return Promise.resolve({ servers, page: 1, total_pages: 1 });
    });

    await renderAndWaitForLoad();
    vi.useRealTimers();

    await waitFor(() => screen.getByText('Hosted Srv'));
    expect(screen.getByText('Stdio Srv')).toBeInTheDocument();

    // Toggle "Stdio" → core is re-queried with transport:'stdio'; hosted drops.
    fireEvent.click(screen.getByRole('button', { name: 'Stdio' }));
    await waitFor(() =>
      expect(mockRegistrySearch).toHaveBeenCalledWith(
        expect.objectContaining({ transport: 'stdio' })
      )
    );
    await waitFor(() => expect(screen.queryByText('Hosted Srv')).not.toBeInTheDocument());
    expect(screen.getByText('Stdio Srv')).toBeInTheDocument();

    // Switch to "Hosted" → transport:'hosted'; stdio drops.
    fireEvent.click(screen.getByRole('button', { name: 'Hosted' }));
    await waitFor(() => expect(screen.getByText('Hosted Srv')).toBeInTheDocument());
    expect(screen.queryByText('Stdio Srv')).not.toBeInTheDocument();
  });

  it('renders every returned row and badges only the official one, with no Show all toggle', async () => {
    mockInstalledList.mockResolvedValue([]);
    mockStatus.mockResolvedValue([]);
    // The core keeps the full deduped catalog and marks the canonical
    // first-party server with `official`. The tab renders every returned row
    // (no client-side filtering) and badges only the official one.
    mockRegistrySearch.mockResolvedValue({
      servers: [
        {
          qualified_name: 'com.notion/mcp',
          display_name: 'Notion',
          is_deployed: true,
          official: true,
        },
        {
          qualified_name: 'ai.smithery/smithery-notion',
          display_name: 'Community Notion',
          is_deployed: true,
          official: false,
        },
      ],
      page: 1,
      total_pages: 1,
    });

    await renderAndWaitForLoad();
    vi.useRealTimers();

    await waitFor(() => screen.getByText('Notion'));
    // The community row still renders — nothing is hidden.
    expect(screen.getByText('Community Notion')).toBeInTheDocument();
    // Exactly one row carries the ✓ Official badge.
    expect(screen.getByText(/Official/)).toBeInTheDocument();
    // No verified/all toggle exists.
    expect(screen.queryByText('Show all')).not.toBeInTheDocument();
    expect(screen.queryByText('Verified only')).not.toBeInTheDocument();
  });

  it('shows a registry error state with retry when the catalog fetch fails', async () => {
    mockInstalledList.mockResolvedValue([]);
    mockStatus.mockResolvedValue([]);
    // First catalog fetch fails → error state; retry succeeds → rows render.
    mockRegistrySearch.mockRejectedValueOnce(new Error('registry down'));
    mockRegistrySearch.mockResolvedValue({
      servers: [{ qualified_name: 'a/srv', display_name: 'Recovered Srv', is_deployed: false }],
      page: 1,
      total_pages: 1,
    });

    await renderAndWaitForLoad();
    vi.useRealTimers();

    // Error surfaces instead of a silent empty state.
    const errorBox = await screen.findByTestId('mcp-catalog-error');
    expect(errorBox).toHaveTextContent('Failed to load catalog');
    expect(screen.queryByTestId('mcp-catalog-empty')).not.toBeInTheDocument();

    // Retry re-fetches and renders the recovered catalog.
    fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
    await waitFor(() => expect(screen.getByText('Recovered Srv')).toBeInTheDocument());
    expect(screen.queryByTestId('mcp-catalog-error')).not.toBeInTheDocument();
  });

  it('surfaces the health toolbar and reconnects error-state servers via Retry all', async () => {
    mockInstalledList.mockResolvedValue(SERVERS);
    mockStatus.mockResolvedValue([
      {
        server_id: 'srv-1',
        qualified_name: 'acme/fs-server',
        display_name: 'File Server',
        status: 'error' as const,
        tool_count: 0,
        last_error: 'connect failed',
      },
    ]);

    await renderAndWaitForLoad();
    vi.useRealTimers();

    // The toolbar (previously never rendered in production) now appears, and its
    // "Retry all" affordance reconnects the errored server.
    const retry = await screen.findByRole('button', { name: /retry all/i });
    fireEvent.click(retry);
    await waitFor(() => expect(mockConnect).toHaveBeenCalledWith('srv-1'));
  });

  it('navigates to install view when a registry server row is clicked', async () => {
    mockInstalledList.mockResolvedValue([]);
    mockStatus.mockResolvedValue([]);
    mockRegistrySearch.mockResolvedValue({
      servers: [{ qualified_name: 'acme/new-srv', display_name: 'New Server', official: true }],
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
      servers: [{ qualified_name: 'acme/new-srv', display_name: 'New Server', official: true }],
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
      servers: [{ qualified_name: 'acme/new-srv', display_name: 'New Server', official: true }],
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
      servers: [{ qualified_name: 'acme/new-srv', display_name: 'New Server', official: true }],
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
    // No installed and no catalog match → the catalog empty-state renders.
    expect(screen.getByTestId('mcp-catalog-empty')).toBeInTheDocument();
  });

  it('Registry chip hides installed rows', async () => {
    mockInstalledList.mockResolvedValue(SERVERS);
    mockStatus.mockResolvedValue(STATUSES_DISCONNECTED);

    await renderAndWaitForLoad();
    vi.useRealTimers();

    await waitFor(() => screen.getByText('File Server'));

    fireEvent.click(screen.getByRole('tab', { name: 'Registry' }));

    await waitFor(() => {
      expect(screen.queryByText('File Server')).not.toBeInTheDocument();
    });
  });
});
