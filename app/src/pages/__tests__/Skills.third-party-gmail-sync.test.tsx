import { fireEvent, screen, within } from '@testing-library/react';
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
    toolkits: ['gmail'],
    connectionByToolkit: new Map([
      ['gmail', { id: 'conn_gmail_1', toolkit: 'gmail', status: 'ACTIVE' }],
    ]),
    refresh: vi.fn(),
    loading: false,
    error: null,
  }),
}));

describe('Skills page — Gmail composio integration', () => {
  it('renders Gmail as a connected composio integration and opens its management modal', async () => {
    renderWithProviders(<Skills />, { initialEntries: ['/skills'] });

    const integrationsSection = screen
      .getByRole('heading', { name: 'Integrations' })
      .closest('.rounded-2xl');
    expect(integrationsSection).not.toBeNull();
    expect(within(integrationsSection as HTMLElement).getByText('Gmail')).toBeInTheDocument();
    expect(within(integrationsSection as HTMLElement).getByText('Connected')).toBeInTheDocument();

    fireEvent.click(
      within(integrationsSection as HTMLElement).getByRole('button', {
        name: /Gmail.*Connected.*Manage/i,
      })
    );

    expect(await screen.findByRole('heading', { name: 'Manage Gmail' })).toBeInTheDocument();
    expect(screen.getByText(/Gmail is connected\./i)).toBeInTheDocument();
  });
});
