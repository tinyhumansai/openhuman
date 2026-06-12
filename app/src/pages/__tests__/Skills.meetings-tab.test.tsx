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

describe('Skills page — Talents tab (meeting bots)', () => {
  it('shows the meeting bot CTA inside the Talents tab (not Tools)', () => {
    renderWithProviders(<Skills />, { initialEntries: ['/connections'] });

    expect(screen.queryByTestId('meeting-bots-card')).not.toBeInTheDocument();

    // Tools no longer hosts the meeting bot CTA.
    fireEvent.click(screen.getByRole('tab', { name: 'Tools' }));
    expect(screen.queryByTestId('meeting-bots-card')).not.toBeInTheDocument();

    // Talents does.
    fireEvent.click(screen.getByRole('tab', { name: 'Talents' }));
    expect(screen.getByTestId('meeting-bots-card')).toBeInTheDocument();
  });

  it('supports direct links via legacy ?tab=meetings (normalised to talents)', () => {
    // The old ?tab=meetings alias now maps to the new "Talents" tab.
    renderWithProviders(<Skills />, { initialEntries: ['/connections?tab=meetings'] });

    expect(screen.getByRole('tab', { name: 'Talents' })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByTestId('meeting-bots-card')).toBeInTheDocument();
  });

  it('supports direct links via ?tab=talents', () => {
    renderWithProviders(<Skills />, { initialEntries: ['/connections?tab=talents'] });

    expect(screen.getByRole('tab', { name: 'Talents' })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByTestId('meeting-bots-card')).toBeInTheDocument();
  });
});
