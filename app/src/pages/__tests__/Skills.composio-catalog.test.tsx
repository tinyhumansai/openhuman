import { screen } from '@testing-library/react';
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
    toolkits: [],
    connectionByToolkit: new Map(),
    refresh: vi.fn(),
    loading: false,
    error: null,
    disabled: false,
  }),
}));

describe('Skills page — Composio catalog fallback', () => {
  it('shows known composio integrations in their configured category groups when the live toolkit list is empty', () => {
    renderWithProviders(<Skills />, { initialEntries: ['/skills'] });

    expect(screen.getByRole('heading', { name: 'Productivity' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Tools & Automation' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Social' })).toBeInTheDocument();
    expect(screen.getByText('Google Calendar')).toBeInTheDocument();
    expect(screen.getByText('Google Drive')).toBeInTheDocument();
    expect(screen.getByText('Gmail')).toBeInTheDocument();
    expect(screen.getByText('Notion')).toBeInTheDocument();
    expect(screen.getByText('GitHub')).toBeInTheDocument();
    expect(screen.getByText('Linear')).toBeInTheDocument();
    expect(screen.getByText('Slack')).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: 'Other' })).not.toBeInTheDocument();
  });
});
