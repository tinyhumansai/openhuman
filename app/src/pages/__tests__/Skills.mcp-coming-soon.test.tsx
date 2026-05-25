import { fireEvent, screen } from '@testing-library/react';
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

describe('Skills page — MCP tab', () => {
  it('shows a coming soon placeholder for MCP server management', () => {
    renderWithProviders(<Skills />, { initialEntries: ['/skills'] });

    fireEvent.click(screen.getByRole('tab', { name: 'MCP Servers' }));

    expect(screen.getAllByRole('heading', { name: 'MCP Servers' })).toHaveLength(2);
    expect(screen.getByText(/MCP server management is coming soon/i)).toBeInTheDocument();
    expect(screen.getByText('Coming Soon')).toBeInTheDocument();
  });
});
