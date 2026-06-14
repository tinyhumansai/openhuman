import { fireEvent, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import '../../test/mockDefaultSkillStatusHooks';
import { renderWithProviders } from '../../test/test-utils';
import Skills from '../Skills';

vi.mock('../../components/skills/MeetingBotsCard', () => ({
  default: () => <div data-testid="meeting-bots-card">Meeting bot CTA</div>,
}));

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

describe('Skills page — Meetings tab (meeting bots)', () => {
  it('shows the meeting bot CTA inside the Meetings tab (not MCP Servers)', () => {
    renderWithProviders(<Skills />, { initialEntries: ['/connections'] });

    expect(screen.queryByTestId('meeting-bots-card')).not.toBeInTheDocument();

    // MCP Servers no longer hosts the meeting bot CTA.
    fireEvent.click(screen.getByTestId('two-pane-nav-mcp'));
    expect(screen.queryByTestId('meeting-bots-card')).not.toBeInTheDocument();

    // Meetings does.
    fireEvent.click(screen.getByTestId('two-pane-nav-meetings'));
    expect(screen.getByTestId('meeting-bots-card')).toBeInTheDocument();
  });

  it('supports direct links via legacy ?tab=meetings (normalised to talents)', () => {
    // The old ?tab=meetings alias now maps to the new "Meetings" tab.
    renderWithProviders(<Skills />, { initialEntries: ['/connections?tab=meetings'] });

    expect(screen.getByTestId('two-pane-nav-meetings')).toHaveAttribute('aria-current', 'page');
    expect(screen.getByTestId('meeting-bots-card')).toBeInTheDocument();
  });

  it('supports direct links via ?tab=talents', () => {
    renderWithProviders(<Skills />, { initialEntries: ['/connections?tab=talents'] });

    expect(screen.getByTestId('two-pane-nav-meetings')).toHaveAttribute('aria-current', 'page');
    expect(screen.getByTestId('meeting-bots-card')).toBeInTheDocument();
  });
});
