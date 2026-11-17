import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { mcpClientsApi } from '../../../services/api/mcpClientsApi';
import McpServersPage from './McpServersPage';

const mockInstalledList = vi.fn();
const mockStatus = vi.fn();
const mockRegistrySearch = vi.fn();
const mockConfigGet = vi.fn();
const mockConfigSet = vi.fn();
const mockDisconnect = vi.fn();
const mockSetEnabled = vi.fn();
const mockUninstall = vi.fn();
const mockListTools = vi.fn();
const mockToolCall = vi.fn();

vi.mock('../../../services/api/mcpClientsApi', () => ({
  mcpClientsApi: {
    installedList: (...args: unknown[]) => mockInstalledList(...args),
    status: (...args: unknown[]) => mockStatus(...args),
    registrySearch: (...args: unknown[]) => mockRegistrySearch(...args),
    configGet: (...args: unknown[]) => mockConfigGet(...args),
    configSet: (...args: unknown[]) => mockConfigSet(...args),
    connect: vi.fn(),
    disconnect: (...args: unknown[]) => mockDisconnect(...args),
    uninstall: (...args: unknown[]) => mockUninstall(...args),
    setEnabled: (...args: unknown[]) => mockSetEnabled(...args),
    listTools: (...args: unknown[]) => mockListTools(...args),
    toolCall: (...args: unknown[]) => mockToolCall(...args),
    updateEnv: vi.fn(),
    detectAuth: vi.fn().mockResolvedValue({ kind: 'none', grant_types: [] }),
    registryGet: vi.fn().mockResolvedValue({ connections: [], required_env_keys: [] }),
  },
}));

vi.mock('../../../utils/openUrl', () => ({ openUrl: vi.fn().mockResolvedValue(undefined) }));

const LOCAL = {
  server_id: 'srv-local',
  qualified_name: 'echo',
  display_name: 'echo',
  command_kind: 'node' as const,
  command: 'npx',
  args: ['-y', 'echo'],
  env_keys: [],
  installed_at: 1,
  transport: { kind: 'stdio' as const },
  enabled: true,
};
const HOSTED = {
  server_id: 'srv-hosted',
  qualified_name: 'hosted',
  display_name: 'hosted',
  command_kind: 'node' as const,
  command: '',
  args: [],
  env_keys: ['Authorization'],
  installed_at: 2,
  transport: { kind: 'http_remote' as const, url: 'https://h.test/mcp' },
  enabled: false,
};

describe('McpServersPage', () => {
  beforeEach(() => {
    mockInstalledList.mockReset();
    mockStatus.mockReset();
    mockRegistrySearch.mockReset();
    mockConfigGet.mockReset();
    mockConfigSet.mockReset();
    mockDisconnect.mockReset();
    mockSetEnabled.mockReset();
    mockUninstall.mockReset();
    mockInstalledList.mockResolvedValue([LOCAL, HOSTED]);
    mockStatus.mockResolvedValue([{ server_id: 'srv-local', status: 'connected', tool_count: 3 }]);
    mockRegistrySearch.mockResolvedValue({ servers: [], page: 1, total_pages: 1 });
    mockConfigGet.mockResolvedValue({ mcpServers: {} });
    mockDisconnect.mockResolvedValue({ status: 'disconnected' });
    mockSetEnabled.mockResolvedValue({ enabled: true });
    mockUninstall.mockResolvedValue({ removed: true });
    mockListTools.mockReset();
    mockListTools.mockResolvedValue([
      { name: 'echo', description: 'Echoes the input', input_schema: {} },
    ]);
    mockToolCall.mockReset();
    mockToolCall.mockResolvedValue({ result: 'echoed: hi', is_error: false });
  });

  it("lists a connected server's tools under its row and opens the playground", async () => {
    render(<McpServersPage />);
    await screen.findByTestId('mcp-servers-section');
    // Only the connected row offers its tools.
    expect(screen.queryByRole('button', { name: "Show hosted's tools" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: "Show echo's tools" }));
    const list = await screen.findByTestId('mcp-row-tools');
    expect(mockListTools).toHaveBeenCalledWith('srv-local');
    expect(list).toHaveTextContent('echo');
    expect(list).toHaveTextContent('Echoes the input');

    fireEvent.click(screen.getByRole('button', { name: 'Open execution playground for echo' }));
    expect(await screen.findByRole('dialog')).toHaveTextContent('Run echo');
  });

  it('puts the three notations in the page header and opens on the rows', async () => {
    render(<McpServersPage />);
    expect(screen.getByRole('heading', { level: 1, name: 'MCP Servers' })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'Servers' })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'mcp.json' })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'Registry' })).toBeInTheDocument();

    await screen.findByTestId('mcp-servers-section');
    const rows = screen.getAllByTestId('mcp-installed-row');
    expect(rows).toHaveLength(2);
    // Each row says how it runs and how it is doing, not where it was found.
    expect(screen.getByText('npx -y echo')).toBeInTheDocument();
    expect(screen.getByText('https://h.test/mcp')).toBeInTheDocument();
    // Status and tool count are now separate table columns rather than one
    // combined string.
    const statusBadges = screen.getAllByTestId('mcp-row-status');
    expect(statusBadges[0]).toHaveTextContent('Connected');
    expect(rows[0]).toHaveTextContent('3');
    expect(screen.getByTestId('mcp-disabled-badge')).toBeInTheDocument();
    // Nothing from the directory sits among the user's own rows.
    expect(mockRegistrySearch).not.toHaveBeenCalled();
  });

  it('switches to the document and the directory, each rendered on demand', async () => {
    render(<McpServersPage />);
    await screen.findByTestId('mcp-servers-section');

    fireEvent.click(screen.getByRole('tab', { name: 'mcp.json' }));
    await screen.findByTestId('mcp-json-editor');
    expect(mockConfigGet).toHaveBeenCalled();
    expect(screen.queryByTestId('mcp-servers-section')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('tab', { name: 'Registry' }));
    await screen.findByTestId('mcp-registry-browser');
    await waitFor(() => expect(mockRegistrySearch).toHaveBeenCalled());
  });

  it('honours an initial tab', async () => {
    render(<McpServersPage initialTab="registry" />);
    await screen.findByTestId('mcp-registry-browser');
  });

  it('re-reads the rows after the document is saved', async () => {
    mockConfigGet.mockResolvedValue({ mcpServers: { echo: { command: 'npx' } } });
    mockConfigSet.mockResolvedValue({
      mcpServers: {},
      added: [],
      updated: [],
      removed: ['echo', 'hosted'],
    });
    render(<McpServersPage />);
    await screen.findByTestId('mcp-servers-section');
    expect(mockInstalledList).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole('tab', { name: 'mcp.json' }));
    await screen.findByTestId('mcp-json-editor');
    mockInstalledList.mockResolvedValue([]);
    fireEvent.change(screen.getByTestId('mcp-json-textarea'), {
      target: { value: '{ "mcpServers": {} }' },
    });
    fireEvent.click(screen.getByTestId('mcp-json-save'));

    await waitFor(() => expect(mockInstalledList).toHaveBeenCalledTimes(2));
    fireEvent.click(screen.getByRole('tab', { name: 'Servers' }));
    await screen.findByTestId('mcp-installed-empty');
  });

  it('re-reads the rows after a server is added from the Registry', async () => {
    const NEW = {
      ...HOSTED,
      server_id: 'srv-new',
      qualified_name: 'com.example/new',
      display_name: 'com.example/new',
      env_keys: [],
      transport: { kind: 'http_remote' as const, url: 'https://new.test/mcp' },
      enabled: true,
    };
    mockRegistrySearch.mockResolvedValue({
      servers: [
        {
          qualified_name: 'com.example/new',
          display_name: 'New Hosted',
          is_deployed: true,
          source: 'mcp_official',
        },
      ],
      page: 1,
      total_pages: 1,
    });
    vi.mocked(mcpClientsApi.registryGet).mockResolvedValueOnce({
      qualified_name: 'com.example/new',
      display_name: 'New Hosted',
      connections: [{ type: 'http', deployment_url: 'https://new.test/mcp' }],
      required_env_keys: [],
    });
    mockConfigSet.mockResolvedValue({
      mcpServers: {},
      added: ['com.example/new'],
      updated: [],
      removed: [],
    });
    render(<McpServersPage initialTab="registry" />);
    fireEvent.click(await screen.findByRole('button', { name: 'Add New Hosted' }));
    const callsBefore = mockInstalledList.mock.calls.length;
    mockInstalledList.mockResolvedValue([LOCAL, HOSTED, NEW]);

    await waitFor(() => expect(screen.getByRole('button', { name: /Added/ })).toBeDisabled());
    expect(mockConfigSet).toHaveBeenCalledWith({
      mcpServers: { 'com.example/new': { url: 'https://new.test/mcp' } },
    });
    expect(mockInstalledList.mock.calls.length).toBeGreaterThan(callsBefore);

    fireEvent.click(screen.getByRole('tab', { name: 'Servers' }));
    await screen.findByTestId('mcp-servers-section');
    expect(screen.getAllByTestId('mcp-installed-row')).toHaveLength(3);
  });

  it("opens a row's name into its detail view and comes back", async () => {
    render(<McpServersPage />);
    await screen.findByTestId('mcp-servers-section');
    fireEvent.click(screen.getByRole('button', { name: 'Open echo' }));
    expect(await screen.findByRole('button', { name: 'Back to servers' })).toBeInTheDocument();
    // The page's own tabs stay put above the detail.
    expect(screen.getByRole('tab', { name: 'mcp.json' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Back to servers' }));
    await screen.findByTestId('mcp-servers-section');
  });

  it('drives a row from its icon controls', async () => {
    render(<McpServersPage />);
    await screen.findByTestId('mcp-servers-section');

    fireEvent.click(screen.getByRole('button', { name: 'Disconnect echo' }));
    await waitFor(() => expect(mockDisconnect).toHaveBeenCalledWith('srv-local'));

    fireEvent.click(screen.getByRole('button', { name: 'Enable hosted' }));
    await waitFor(() => expect(mockSetEnabled).toHaveBeenCalledWith('srv-hosted', true));

    // Removal asks first, then tells the core.
    fireEvent.click(screen.getByRole('button', { name: 'Remove echo' }));
    expect(await screen.findByTestId('mcp-remove-dialog')).toBeInTheDocument();
    expect(mockUninstall).not.toHaveBeenCalled();
    fireEvent.click(screen.getByTestId('confirm-dialog-confirm'));
    await waitFor(() => expect(mockUninstall).toHaveBeenCalledWith('srv-local'));
  });

  it('routes an empty list to the document', async () => {
    mockInstalledList.mockResolvedValue([]);
    mockStatus.mockResolvedValue([]);
    render(<McpServersPage />);
    await screen.findByTestId('mcp-installed-empty');
    fireEvent.click(screen.getByRole('button', { name: 'Add one in mcp.json' }));
    await screen.findByTestId('mcp-json-editor');
  });
});
