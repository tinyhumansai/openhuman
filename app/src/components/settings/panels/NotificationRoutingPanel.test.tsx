import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

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

const fetchStatsMock = vi.fn();
const getSettingsMock = vi.fn();
const setSettingsMock = vi.fn();
vi.mock('../../../services/notificationService', () => ({
  fetchNotificationStats: () => fetchStatsMock(),
  getNotificationSettings: (provider: string) => getSettingsMock(provider),
  setNotificationSettings: (payload: unknown) => setSettingsMock(payload),
}));

const PROVIDERS = ['gmail', 'slack', 'discord', 'whatsapp'];

async function importPanel() {
  vi.resetModules();
  const mod = await import('./NotificationRoutingPanel');
  return mod.default;
}

function renderPanel(Panel: React.ComponentType) {
  return render(
    <MemoryRouter>
      <Panel />
    </MemoryRouter>
  );
}

function defaultSettings() {
  return { enabled: true, importance_threshold: 0.3, route_to_orchestrator: true };
}

describe('<NotificationRoutingPanel />', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    fetchStatsMock.mockResolvedValue({ total: 12, unread: 5, unscored: 2 });
    getSettingsMock.mockImplementation((provider: string) =>
      Promise.resolve({ provider, ...defaultSettings() })
    );
    setSettingsMock.mockResolvedValue(undefined);
  });

  it('renders the pipeline stats card once fetchNotificationStats resolves', async () => {
    const Panel = await importPanel();
    renderPanel(Panel);

    await waitFor(() => expect(screen.getByText('Pipeline stats')).toBeInTheDocument());
    // total / unread / unscored values appear as separate stat tiles
    expect(screen.getByText('12')).toBeInTheDocument();
    expect(screen.getByText('5')).toBeInTheDocument();
    expect(screen.getByText('2')).toBeInTheDocument();
  });

  it('omits the pipeline stats card when fetchNotificationStats rejects', async () => {
    fetchStatsMock.mockRejectedValueOnce(new Error('stats down'));
    const Panel = await importPanel();
    renderPanel(Panel);

    // Settings still load for each provider so the per-provider section
    // renders, but the stats card is absent.
    await waitFor(() => expect(getSettingsMock).toHaveBeenCalledTimes(PROVIDERS.length));
    expect(screen.queryByText('Pipeline stats')).not.toBeInTheDocument();
  });

  it('renders one row per provider with persisted threshold values', async () => {
    getSettingsMock.mockImplementation((provider: string) =>
      Promise.resolve({
        provider,
        enabled: true,
        importance_threshold: 0.45,
        route_to_orchestrator: true,
      })
    );
    const Panel = await importPanel();
    renderPanel(Panel);

    await waitFor(() => expect(getSettingsMock).toHaveBeenCalledTimes(PROVIDERS.length));
    // Provider headings render capitalized via CSS but the underlying text
    // is lowercased; assert against the rendered text directly.
    for (const provider of PROVIDERS) {
      expect(screen.getByText(provider)).toBeInTheDocument();
    }
    // Each row formats the threshold to 2 decimals — four rows × 0.45.
    expect(screen.getAllByText('0.45')).toHaveLength(PROVIDERS.length);
  });

  it('toggling enabled for a provider fires setNotificationSettings', async () => {
    const Panel = await importPanel();
    renderPanel(Panel);

    await waitFor(() => expect(getSettingsMock).toHaveBeenCalledTimes(PROVIDERS.length));

    // Each provider exposes an "Enabled" checkbox; the first one is gmail's.
    const enabledCheckboxes = screen.getAllByRole('checkbox', { name: 'Enabled' });
    expect(enabledCheckboxes).toHaveLength(PROVIDERS.length);
    fireEvent.click(enabledCheckboxes[0]);

    await waitFor(() =>
      expect(setSettingsMock).toHaveBeenCalledWith({
        provider: 'gmail',
        enabled: false,
        importance_threshold: 0.3,
        route_to_orchestrator: true,
      })
    );
  });

  it('disables the controls and surfaces a retry hint for providers that fail to load', async () => {
    getSettingsMock.mockImplementation((provider: string) => {
      if (provider === 'gmail') return Promise.reject(new Error('boom'));
      return Promise.resolve({ provider, ...defaultSettings() });
    });
    const Panel = await importPanel();
    renderPanel(Panel);

    await waitFor(() =>
      expect(
        screen.getByText('Failed to load settings. Reopen this panel to retry.')
      ).toBeInTheDocument()
    );

    // The four `Enabled` checkboxes still render, but the first row (gmail)
    // is disabled because its settings never loaded.
    const enabledCheckboxes = screen.getAllByRole('checkbox', { name: 'Enabled' });
    expect(enabledCheckboxes[0]).toBeDisabled();
    // The slack row should still be interactive.
    expect(enabledCheckboxes[1]).not.toBeDisabled();
  });

  it('renders the static "How it works" explainer rows', async () => {
    const Panel = await importPanel();
    renderPanel(Panel);

    await waitFor(() => expect(getSettingsMock).toHaveBeenCalled());
    // The four routing tiers are part of the static explainer and should
    // render regardless of network state.
    expect(screen.getByText('Drop')).toBeInTheDocument();
    expect(screen.getByText('Acknowledge')).toBeInTheDocument();
    expect(screen.getByText('React')).toBeInTheDocument();
    expect(screen.getByText('Escalate')).toBeInTheDocument();
  });
});
