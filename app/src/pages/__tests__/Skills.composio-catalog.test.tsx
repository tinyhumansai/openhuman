import { fireEvent, screen, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import '../../test/mockDefaultSkillStatusHooks';
import { renderWithProviders } from '../../test/test-utils';
import Skills from '../Skills';

let composioRefresh = vi.fn();
let composioError: string | null = null;
let composioToolkits: string[] = [];
let composioConnectionByToolkit = new Map();

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
    toolkits: composioToolkits,
    connectionByToolkit: composioConnectionByToolkit,
    refresh: composioRefresh,
    loading: false,
    error: composioError,
  }),
}));

describe('Skills page — Composio catalog fallback', () => {
  beforeEach(() => {
    composioRefresh = vi.fn();
    composioError = null;
    composioToolkits = [];
    composioConnectionByToolkit = new Map();
  });

  it('shows known composio integrations in the integrations icon grid when the live toolkit list is empty', () => {
    renderWithProviders(<Skills />, { initialEntries: ['/skills'] });

    expect(screen.getByRole('heading', { name: 'Integrations' })).toBeInTheDocument();
    expect(screen.getByText('Discord')).toBeInTheDocument();
    expect(screen.getByText('Google Calendar')).toBeInTheDocument();
    expect(screen.getByText('Google Drive')).toBeInTheDocument();
    expect(screen.getByText('Gmail')).toBeInTheDocument();
    expect(screen.getByText('Google Sheets')).toBeInTheDocument();
    expect(screen.getByText('Facebook')).toBeInTheDocument();
    expect(screen.getByText('GitHub')).toBeInTheDocument();
    expect(screen.getByText('Instagram')).toBeInTheDocument();
    expect(screen.getByText('Linear')).toBeInTheDocument();
    expect(screen.getByText('Reddit')).toBeInTheDocument();
    expect(screen.getByText('Slack')).toBeInTheDocument();
    expect(screen.getByText('Supabase')).toBeInTheDocument();
    // Scope to the Integrations section so the assertion still catches a
    // missing Composio Zoom tile even though the Meeting bots card also
    // renders a "Zoom" entry on the same page.
    const integrationsSection = screen
      .getByRole('heading', { name: 'Integrations' })
      .closest('.rounded-2xl');
    expect(integrationsSection).not.toBeNull();
    expect(within(integrationsSection as HTMLElement).getByText('Zoom')).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: 'Other' })).not.toBeInTheDocument();
  });

  it('shows a stale/error state instead of disconnected toolkits when composio loading fails', () => {
    composioError = 'Backend unavailable';

    renderWithProviders(<Skills />, { initialEntries: ['/skills'] });

    expect(screen.getByText('Connections are showing stale status')).toBeInTheDocument();
    expect(screen.getByText('Backend unavailable')).toBeInTheDocument();

    const integrationsSection = screen
      .getByRole('heading', { name: 'Integrations' })
      .closest('.rounded-2xl');
    expect(integrationsSection).not.toBeNull();
    const gmailTile = within(integrationsSection as HTMLElement).getByRole('button', {
      name: /Gmail.*Status unavailable/i,
    });
    expect(gmailTile).toBeInTheDocument();
    expect(within(gmailTile).getByText('Status unavailable')).toBeInTheDocument();

    fireEvent.click(screen.getAllByRole('button', { name: 'Retry' })[0]);
    expect(composioRefresh).toHaveBeenCalledTimes(1);
  });

  it('surfaces expired Composio auth as reconnectable from the Gmail tile', () => {
    composioToolkits = ['gmail'];
    composioConnectionByToolkit = new Map([
      ['gmail', { id: 'ca_expired', toolkit: 'gmail', status: 'EXPIRED' }],
    ]);

    renderWithProviders(<Skills />, { initialEntries: ['/skills'] });

    const integrationsSection = screen
      .getByRole('heading', { name: 'Integrations' })
      .closest('.rounded-2xl');
    expect(integrationsSection).not.toBeNull();
    const gmailTile = within(integrationsSection as HTMLElement).getByRole('button', {
      name: /Gmail.*Auth expired.*Reconnect/i,
    });

    expect(within(gmailTile).getByText('Auth expired')).toBeInTheDocument();

    fireEvent.click(gmailTile);

    expect(screen.getByText(/Gmail authorization expired/i)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Reconnect Gmail/i })).toBeInTheDocument();
  });
});
