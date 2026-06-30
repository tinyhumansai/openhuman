import { fireEvent, screen, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

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

// The built-in web channel needs no connection, so the grid always renders it
// as connected — making it the reliable "connected, not default" tile.
const webDef: ChannelDefinition = {
  id: 'web',
  display_name: 'Web',
  description: 'Chat via the built-in web UI.',
  icon: 'web',
  auth_modes: [],
  capabilities: [],
};

vi.mock('../../hooks/useChannelDefinitions', () => ({
  useChannelDefinitions: () => ({
    definitions: [telegramDef, imessageDef, webDef],
    loading: false,
    error: null,
  }),
}));

const { updatePreferencesMock } = vi.hoisted(() => ({
  updatePreferencesMock: vi.fn<(channel: string) => Promise<void>>(),
}));

vi.mock('../../services/api/channelConnectionsApi', () => ({
  channelConnectionsApi: { updatePreferences: (channel: string) => updatePreferencesMock(channel) },
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
  // Issue #2283: Skills.tsx also consumes useAgentReadyComposioToolkits.
  // `loading: true` keeps Preview badges off so legacy aria-label
  // assertions on this page keep passing.
  useAgentReadyComposioToolkits: () => ({
    agentReady: new Set<string>(),
    loading: true,
    error: null,
  }),
}));

describe('Skills page — Channels grid', () => {
  beforeEach(() => {
    // The default tab is 'composio'; click 'Messaging' to reveal the Channels card.
    updatePreferencesMock.mockReset();
    updatePreferencesMock.mockResolvedValue(undefined);
  });

  it('groups connected channels ahead of not-connected ones', () => {
    const preloadedState = {
      channelConnections: {
        schemaVersion: 1,
        migrationCompleted: true,
        defaultMessagingChannel: 'web' as const,
        connections: {
          telegram: {
            managed_dm: undefined,
            oauth: {
              channel: 'telegram' as const,
              authMode: 'oauth' as const,
              status: 'connected' as const,
              selectedDefault: false,
              lastError: null,
              capabilities: [],
              updatedAt: new Date().toISOString(),
            },
            bot_token: undefined,
            api_key: undefined,
          },
        },
      },
    };

    renderWithProviders(<Skills />, { initialEntries: ['/connections'], preloadedState });
    fireEvent.click(screen.getByTestId('two-pane-nav-channels'));

    const channelsCard = screen.getByRole('heading', { name: 'Messaging' }).closest('.rounded-2xl');
    const order = within(channelsCard as HTMLElement)
      .getAllByTestId(/^skill-row-channel-/)
      .map(el => el.getAttribute('data-testid'));

    // Connected channels (Telegram + always-on Web) come before the
    // disconnected one (iMessage), with no separator between the groups.
    const idxTelegram = order.indexOf('skill-row-channel-telegram');
    const idxWeb = order.indexOf('skill-row-channel-web');
    const idxImessage = order.indexOf('skill-row-channel-imessage');
    expect(idxTelegram).toBeGreaterThanOrEqual(0);
    expect(idxWeb).toBeGreaterThanOrEqual(0);
    expect(idxImessage).toBeGreaterThan(idxTelegram);
    expect(idxImessage).toBeGreaterThan(idxWeb);
  });

  it('offers "set as default" only on connected tiles and switches the default', async () => {
    const { store } = renderWithProviders(<Skills />, { initialEntries: ['/connections'] });
    fireEvent.click(screen.getByTestId('two-pane-nav-channels'));

    const channelsCard = screen.getByRole('heading', { name: 'Messaging' }).closest('.rounded-2xl');
    const within$ = within(channelsCard as HTMLElement);

    // The redux default starts on Telegram, so its tile shows the "Default"
    // badge (there is no longer a second, separate channel-picker list below).
    expect(within$.getByTestId('channel-select-telegram')).toHaveTextContent('Default');
    // Web is always connected but not the default → offers "Set as default".
    const setWebDefault = within$.getByTestId('channel-select-web');
    expect(setWebDefault).toHaveTextContent('Set as default');
    // A disconnected, non-default channel offers no default control at all.
    expect(within$.queryByTestId('channel-select-imessage')).toBeNull();

    fireEvent.click(setWebDefault);

    await vi.waitFor(() =>
      expect(store.getState().channelConnections.defaultMessagingChannel).toBe('web')
    );
    expect(updatePreferencesMock).toHaveBeenCalledWith('web');
  });

  it('renders configured channels as tiles in a dedicated card and opens the setup modal on click', async () => {
    renderWithProviders(<Skills />, { initialEntries: ['/connections'] });

    // Switch to the Channels tab to make the Channels card visible.
    fireEvent.click(screen.getByTestId('two-pane-nav-channels'));

    const channelsHeading = screen.getByRole('heading', { name: 'Messaging' });
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

      renderWithProviders(<Skills />, { initialEntries: ['/connections'], preloadedState });
      // Switch to the Channels tab so the Channels card is visible.
      fireEvent.click(screen.getByTestId('two-pane-nav-channels'));
      const channelsCard = screen
        .getByRole('heading', { name: 'Messaging' })
        .closest('.rounded-2xl');
      const telegramTile = within(channelsCard as HTMLElement).getByRole('button', {
        name: new RegExp(`Telegram.*${labelPattern.source}`, 'i'),
      });
      // The connection-status colour now lives on the tile container (the
      // inner button only owns the "configure" affordance), so assert against
      // the wrapping tile rather than the button itself.
      const tileContainer = telegramTile.closest('.rounded-2xl');
      expect(tileContainer).not.toBeNull();
      expect((tileContainer as HTMLElement).className).toMatch(classPattern);
    }
  );

  it('does not surface a Channels chip in the category filter inside the Integrations card', () => {
    renderWithProviders(<Skills />, { initialEntries: ['/connections'] });
    fireEvent.click(screen.getByTestId('two-pane-nav-composio'));

    // The Composio tab owns the Integrations category filter.
    const integrationsCard = screen.getByTestId('composio-integrations-card');
    const filterTabs = within(integrationsCard as HTMLElement)
      .queryAllByRole('tab')
      .map(el => el.textContent?.trim());
    expect(filterTabs).not.toContain('Channels');
  });
});
