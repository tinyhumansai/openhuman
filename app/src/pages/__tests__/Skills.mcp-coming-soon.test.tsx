import { fireEvent, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import '../../test/mockDefaultSkillStatusHooks';
import { renderWithProviders } from '../../test/test-utils';
import Skills from '../Skills';

vi.mock('../../hooks/useChannelDefinitions', () => ({
  useChannelDefinitions: () => ({ definitions: [], loading: false, error: null }),
}));

vi.mock('../../services/api/workflowsApi', async () => {
  const actual = await vi.importActual<typeof import('../../services/api/workflowsApi')>(
    '../../services/api/workflowsApi'
  );
  return {
    ...actual,
    workflowsApi: { ...actual.workflowsApi, listWorkflows: vi.fn().mockResolvedValue([]) },
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
    install: vi.fn().mockResolvedValue({}),
    connect: vi.fn().mockResolvedValue({ tools: [] }),
    disconnect: vi.fn().mockResolvedValue({}),
    uninstall: vi.fn().mockResolvedValue({}),
    configAssist: vi.fn().mockResolvedValue({}),
  },
}));

describe('Skills page — MCP Servers tab (MCP + Meeting bots)', () => {
  it('renders the MCP servers table in the MCP Servers tab', async () => {
    renderWithProviders(<Skills />, { initialEntries: ['/connections'] });

    fireEvent.click(screen.getByTestId('two-pane-nav-mcp'));

    // The Tools tab shows filter chips (All / Installed / Registry) and a search input
    await waitFor(() => {
      expect(screen.getByRole('tab', { name: 'All' })).toBeInTheDocument();
    });
    expect(screen.getByRole('tab', { name: /Installed/i })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'Registry' })).toBeInTheDocument();
  });

  it('shows the table header columns on the MCP Servers tab', async () => {
    renderWithProviders(<Skills />, { initialEntries: ['/connections'] });

    fireEvent.click(screen.getByTestId('two-pane-nav-mcp'));

    // Wait for initial load to complete
    await waitFor(() => {
      expect(screen.queryByText('Loading MCP servers...')).not.toBeInTheDocument();
    });

    expect(screen.getByText('Name')).toBeInTheDocument();
    expect(screen.getByText('Author')).toBeInTheDocument();
    expect(screen.getByText('Action')).toBeInTheDocument();
  });

  it('shows empty-installed state when Installed chip is clicked', async () => {
    renderWithProviders(<Skills />, { initialEntries: ['/connections'] });

    fireEvent.click(screen.getByTestId('two-pane-nav-mcp'));

    await waitFor(() => {
      expect(screen.getByRole('tab', { name: /Installed/i })).toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole('tab', { name: /Installed/i }));

    await waitFor(() => {
      expect(screen.getByText('No MCP servers installed yet.')).toBeInTheDocument();
    });
  });

  it('supports direct links via legacy ?tab=mcp (normalised to mcp-servers)', async () => {
    renderWithProviders(<Skills />, { initialEntries: ['/connections?tab=mcp'] });

    expect(screen.getByTestId('two-pane-nav-mcp')).toHaveAttribute('aria-current', 'page');
    await waitFor(() => {
      expect(screen.getByRole('tab', { name: 'All' })).toBeInTheDocument();
    });
  });
});
