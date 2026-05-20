import { configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import channelConnectionsReducer from '../../../store/channelConnectionsSlice';

const navigateBack = vi.fn();

vi.mock('../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack,
    navigateToSettings: vi.fn(),
    navigateToTeamManagement: vi.fn(),
    breadcrumbs: [],
  }),
}));

vi.mock('../components/SettingsHeader', () => ({
  default: ({ title }: { title: string }) => <div data-testid="settings-header">{title}</div>,
}));

vi.mock('../../channels/ChannelSetupModal', () => ({
  default: ({
    definition,
    onClose,
  }: {
    definition: { id: string; display_name: string };
    onClose: () => void;
  }) => (
    <div data-testid="channel-setup-modal" data-channel={definition.id}>
      <p>Setup: {definition.display_name}</p>
      <button type="button" onClick={onClose}>
        close
      </button>
    </div>
  ),
}));

const useChannelDefinitionsMock = vi.fn();
vi.mock('../../../hooks/useChannelDefinitions', () => ({
  useChannelDefinitions: () => useChannelDefinitionsMock(),
}));

const updatePreferencesMock = vi.fn();
vi.mock('../../../services/api/channelConnectionsApi', () => ({
  channelConnectionsApi: { updatePreferences: (channel: string) => updatePreferencesMock(channel) },
}));

const FIXTURE_DEFINITIONS = [
  { id: 'telegram', display_name: 'Telegram', description: 'Chat via Telegram', icon: 'telegram' },
  { id: 'discord', display_name: 'Discord', description: 'Chat via Discord', icon: 'discord' },
  { id: 'web', display_name: 'Web', description: 'Browser-based chat', icon: 'web' },
];

function buildStore(defaultChannel: 'telegram' | 'discord' | 'web' = 'telegram') {
  const preloadedState = {
    channelConnections: {
      defaultMessagingChannel: defaultChannel,
      connections: { telegram: {}, discord: {}, web: {} },
      migrationCompleted: true,
    },
  } as unknown as Parameters<typeof configureStore>[0]['preloadedState'];
  return configureStore({
    reducer: { channelConnections: channelConnectionsReducer },
    preloadedState,
  });
}

async function importPanel() {
  vi.resetModules();
  const mod = await import('./MessagingPanel');
  return mod.default;
}

function renderPanel(Panel: React.ComponentType, store = buildStore()) {
  return render(
    <Provider store={store}>
      <MemoryRouter>
        <Panel />
      </MemoryRouter>
    </Provider>
  );
}

describe('<MessagingPanel />', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useChannelDefinitionsMock.mockReturnValue({
      definitions: FIXTURE_DEFINITIONS,
      loading: false,
      error: null,
      refreshDefinitions: vi.fn(),
    });
    updatePreferencesMock.mockResolvedValue(undefined);
  });

  it('shows the loading state from useChannelDefinitions', async () => {
    useChannelDefinitionsMock.mockReturnValue({
      definitions: [],
      loading: true,
      error: null,
      refreshDefinitions: vi.fn(),
    });
    const Panel = await importPanel();
    renderPanel(Panel);

    expect(screen.getByText('Loading channel definitions...')).toBeInTheDocument();
  });

  it('surfaces the load error returned by the channel definitions hook', async () => {
    useChannelDefinitionsMock.mockReturnValue({
      definitions: [],
      loading: false,
      error: 'backend unreachable',
      refreshDefinitions: vi.fn(),
    });
    const Panel = await importPanel();
    renderPanel(Panel);

    expect(screen.getByText('backend unreachable')).toBeInTheDocument();
  });

  it('renders one button per definition for the default selector and excludes `web` from connections', async () => {
    const Panel = await importPanel();
    renderPanel(Panel);

    // The default selector shows ALL definitions (telegram, discord, web).
    const defaultButtons = screen.getAllByRole('button', { name: /^Telegram$|^Discord$|^Web$/ });
    expect(defaultButtons.map(btn => btn.textContent)).toEqual(
      expect.arrayContaining(['Telegram', 'Discord', 'Web'])
    );

    // The "Channel Connections" cards filter out `web` per the panel's
    // configurableChannels memo, so the configurable rows are only
    // telegram + discord.
    const connectionRows = screen.getAllByRole('button', { name: /Chat via (Telegram|Discord)/ });
    expect(connectionRows).toHaveLength(2);
  });

  it('opens the ChannelSetupModal for the clicked channel and closes it', async () => {
    const Panel = await importPanel();
    renderPanel(Panel);

    const telegramRow = screen.getByRole('button', { name: /Chat via Telegram/ });
    fireEvent.click(telegramRow);

    const modal = await screen.findByTestId('channel-setup-modal');
    expect(modal).toHaveAttribute('data-channel', 'telegram');

    fireEvent.click(screen.getByRole('button', { name: 'close' }));
    await waitFor(() => {
      expect(screen.queryByTestId('channel-setup-modal')).not.toBeInTheDocument();
    });
  });

  it('clicking a default-channel button calls updatePreferences for that channel', async () => {
    const Panel = await importPanel();
    renderPanel(Panel);

    // discord is not currently the default; clicking selects it.
    fireEvent.click(screen.getByRole('button', { name: 'Discord' }));
    await waitFor(() => expect(updatePreferencesMock).toHaveBeenCalledWith('discord'));
  });
});
