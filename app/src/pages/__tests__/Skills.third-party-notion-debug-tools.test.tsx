import { fireEvent, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import '../../test/mockDefaultSkillStatusHooks';
import { renderWithProviders } from '../../test/test-utils';
import Skills from '../Skills';

vi.mock('../../hooks/useChannelDefinitions', () => ({
  useChannelDefinitions: () => ({ definitions: [], loading: false, error: null }),
}));

vi.mock('../../lib/skills/skillsApi', () => ({
  installSkill: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('../../lib/skills/hooks', () => ({
  useAvailableSkills: () => ({ skills: [], loading: false, refresh: vi.fn() }),
}));

vi.mock('../../lib/composio/hooks', () => ({
  useComposioIntegrations: () => ({
    toolkits: ['notion'],
    connectionByToolkit: new Map(),
    refresh: vi.fn(),
    loading: false,
    error: null,
  }),
}));

describe('Skills page — Notion composio integration', () => {
  it('renders Notion as a disconnected composio integration and opens its connect modal', async () => {
    renderWithProviders(<Skills />, { initialEntries: ['/skills'] });

    expect(screen.getByRole('heading', { name: 'Integrations' })).toBeInTheDocument();
    const notionTile = screen.getByRole('button', { name: /Notion.*Connect/i });
    expect(notionTile).toBeInTheDocument();

    fireEvent.click(notionTile);

    expect(await screen.findByRole('heading', { name: 'Connect Notion' })).toBeInTheDocument();
    expect(screen.getByText(/Connect your Notion account\./i)).toBeInTheDocument();
    expect(screen.getByText(/OpenHuman's own agent permissions/i)).toBeInTheDocument();
  });
});
