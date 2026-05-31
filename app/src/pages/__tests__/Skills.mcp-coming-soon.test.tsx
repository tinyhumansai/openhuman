import { fireEvent, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import '../../test/mockDefaultSkillStatusHooks';
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
    skillsApi: { ...actual.skillsApi, listSkills: vi.fn().mockResolvedValue([]) },
  };
});

vi.mock('../../lib/composio/hooks', () => ({
  useComposioIntegrations: () => ({
    toolkits: [],
    connectionByToolkit: new Map(),
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

describe('Skills page — MCP tab', () => {
  it('renders the live MCP servers tab (not a coming-soon placeholder)', async () => {
    renderWithProviders(<Skills />, { initialEntries: ['/skills'] });

    fireEvent.click(screen.getByRole('tab', { name: 'MCP Servers' }));

    await waitFor(() => {
      expect(
        screen.getByText('No MCP servers installed yet.') ||
          screen.getByText('Loading MCP servers...')
      ).toBeInTheDocument();
    });
  });
});
