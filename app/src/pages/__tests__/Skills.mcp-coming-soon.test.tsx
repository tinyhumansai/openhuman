import { fireEvent, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import Skills from '../Skills';

vi.mock('../../hooks/useChannelDefinitions', () => ({
  useChannelDefinitions: () => ({ definitions: [], loading: false, error: null }),
}));

vi.mock('../../services/api/skillsApi', async () => {
  const actual = await vi.importActual<typeof import('../../services/api/skillsApi')>(
    '../../services/api/skillsApi'
  );
  return {
    ...actual,
    skillsApi: { ...actual.skillsApi, listWorkflows: vi.fn().mockResolvedValue([]) },
  };
});

vi.mock('../../lib/composio/hooks', () => ({
  useComposioIntegrations: () => ({
    toolkits: [],
    connectionByToolkit: new Map(),
    connectionsByToolkit: new Map(),
    refresh: vi.fn(),
    loading: false,
    error: null,
  }),
  useAgentReadyComposioToolkits: () => ({
    agentReady: new Set<string>(),
    loading: true,
    error: null,
  }),
}));

vi.mock('../../services/api/mcpClientsApi', () => ({
  mcpClientsApi: {
    installedList: vi.fn().mockResolvedValue([]),
    status: vi.fn().mockResolvedValue([]),
    registrySearch: vi.fn().mockResolvedValue({ servers: [], page: 1, total_pages: 1 }),
    registryGet: vi.fn().mockResolvedValue(null),
    configGet: vi.fn().mockResolvedValue({ mcpServers: {} }),
    configSet: vi.fn().mockResolvedValue({ mcpServers: {}, added: [], updated: [], removed: [] }),
    connect: vi.fn().mockResolvedValue({ tools: [] }),
    disconnect: vi.fn().mockResolvedValue({}),
    uninstall: vi.fn().mockResolvedValue({}),
  },
}));

describe('Skills page — MCP Servers tab (MCP + Meeting bots)', () => {
  it('renders the MCP servers table in the MCP Servers tab', async () => {
    renderWithProviders(<Skills />, { initialEntries: ['/connections'] });

    fireEvent.click(screen.getByTestId('two-pane-nav-mcp'));

    // The MCP page is three notations: the server rows, the document, the
    // directory.
    await waitFor(() => {
      expect(screen.getByRole('tab', { name: 'Servers' })).toBeInTheDocument();
    });
    expect(screen.getByRole('tab', { name: 'mcp.json' })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'Registry' })).toBeInTheDocument();
  });

  it('renders the page header with the rows section beneath it', async () => {
    renderWithProviders(<Skills />, { initialEntries: ['/connections'] });

    fireEvent.click(screen.getByTestId('two-pane-nav-mcp'));

    // Wait for initial load to complete
    await waitFor(() => {
      expect(screen.queryByText('Loading MCP servers...')).not.toBeInTheDocument();
    });

    expect(screen.getByRole('heading', { level: 1, name: 'MCP Servers' })).toBeInTheDocument();
    // The rows table's own title ("Installed servers") is the DataTable
    // card's h3, not a page-level h2.
    expect(
      screen.getByRole('heading', { level: 3, name: 'Installed servers' })
    ).toBeInTheDocument();
  });

  it('shows the empty state with a route to mcp.json when nothing is declared', async () => {
    renderWithProviders(<Skills />, { initialEntries: ['/connections'] });

    fireEvent.click(screen.getByTestId('two-pane-nav-mcp'));

    await waitFor(() => {
      expect(screen.getByText('No MCP servers installed yet.')).toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole('button', { name: 'Add one in mcp.json' }));
    await waitFor(() => {
      expect(screen.getByTestId('mcp-json-editor')).toBeInTheDocument();
    });
  });

  it('supports direct links via legacy ?tab=mcp (normalised to mcp-servers)', async () => {
    renderWithProviders(<Skills />, { initialEntries: ['/connections?tab=mcp'] });

    expect(screen.getByTestId('two-pane-nav-mcp')).toHaveAttribute('aria-current', 'page');
    await waitFor(() => {
      expect(screen.getByRole('tab', { name: 'Servers' })).toBeInTheDocument();
    });
  });
});
