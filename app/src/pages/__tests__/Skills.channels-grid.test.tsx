import { fireEvent, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import '../../test/mockDefaultSkillStatusHooks';
import { renderWithProviders } from '../../test/test-utils';
import type { ChannelDefinition } from '../../types/channels';
import Skills from '../Skills';

const telegramDef: ChannelDefinition = {
  id: 'telegram',
  display_name: 'Telegram',
  description: 'Send and receive messages on Telegram.',
  icon: 'telegram',
  auth_modes: [],
  capabilities: [],
};

const imessageDef: ChannelDefinition = {
  id: 'imessage',
  display_name: 'iMessage',
  description: 'Reach iMessage threads on macOS.',
  icon: 'imessage',
  auth_modes: [],
  capabilities: [],
};

vi.mock('../../hooks/useChannelDefinitions', () => ({
  useChannelDefinitions: () => ({
    definitions: [telegramDef, imessageDef],
    loading: false,
    error: null,
  }),
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
}));

describe('Skills page — Channels grid', () => {
  it('renders configured channels as tiles in a dedicated card and opens the setup modal on click', async () => {
    renderWithProviders(<Skills />, { initialEntries: ['/skills'] });

    const channelsHeading = screen.getByRole('heading', { name: 'Channels' });
    expect(channelsHeading).toBeInTheDocument();

    const channelsCard = channelsHeading.closest('.rounded-2xl');
    expect(channelsCard).not.toBeNull();
    const within$ = within(channelsCard as HTMLElement);

    const telegramTile = within$.getByRole('button', { name: /Telegram.*Not configured.*Setup/i });
    expect(telegramTile).toBeInTheDocument();
    const imessageTile = within$.getByRole('button', { name: /iMessage.*Not configured.*Setup/i });
    expect(imessageTile).toBeInTheDocument();

    fireEvent.click(telegramTile);
    const dialog = await screen.findByRole('dialog');
    expect(
      within(dialog).getByText(/Send and receive messages on Telegram\./i)
    ).toBeInTheDocument();
  });

  it.each([
    ['connected', /Connected/i, /sage/],
    ['connecting', /Connecting/i, /amber/],
    ['error', /Error/i, /coral/],
  ] as const)(
    'styles the Telegram channel tile to reflect the %s connection state',
    (status, labelPattern, classPattern) => {
      const preloadedState = {
        channelConnections: {
          schemaVersion: 1,
          migrationCompleted: true,
          defaultMessagingChannel: 'telegram' as const,
          connections: {
            telegram: {
              managed_dm: undefined,
              oauth: {
                channel: 'telegram' as const,
                authMode: 'oauth' as const,
                status,
                selectedDefault: false,
                lastError: null,
                capabilities: [],
                updatedAt: new Date().toISOString(),
              },
              bot_token: undefined,
              api_key: undefined,
            },
            discord: {
              managed_dm: undefined,
              oauth: undefined,
              bot_token: undefined,
              api_key: undefined,
            },
            web: {
              managed_dm: undefined,
              oauth: undefined,
              bot_token: undefined,
              api_key: undefined,
            },
          },
        },
      };

      renderWithProviders(<Skills />, { initialEntries: ['/skills'], preloadedState });
      const channelsCard = screen
        .getByRole('heading', { name: 'Channels' })
        .closest('.rounded-2xl');
      const telegramTile = within(channelsCard as HTMLElement).getByRole('button', {
        name: new RegExp(`Telegram.*${labelPattern.source}`, 'i'),
      });
      expect(telegramTile.className).toMatch(classPattern);
    }
  );

  it('does not surface a Channels chip in the category filter inside the Integrations card', () => {
    renderWithProviders(<Skills />, { initialEntries: ['/skills'] });

    const integrationsHeading = screen.getByRole('heading', { name: 'Integrations' });
    const integrationsCard = integrationsHeading.closest('.rounded-2xl');
    expect(integrationsCard).not.toBeNull();
    const filterTabs = within(integrationsCard as HTMLElement)
      .queryAllByRole('tab')
      .map(el => el.textContent?.trim());
    expect(filterTabs).not.toContain('Channels');
  });
});
