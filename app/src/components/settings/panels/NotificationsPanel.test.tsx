import { configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import notificationReducer, { type NotificationState } from '../../../store/notificationSlice';

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

const getBypassPrefsMock = vi.fn();
const setGlobalDndMock = vi.fn();
vi.mock('../../../services/webviewAccountService', () => ({
  getBypassPrefs: () => getBypassPrefsMock(),
  setGlobalDnd: (enabled: boolean) => setGlobalDndMock(enabled),
}));

function buildStore(preloaded?: Partial<NotificationState>) {
  return configureStore({
    reducer: { notifications: notificationReducer },
    preloadedState: preloaded
      ? {
          notifications: {
            items: [],
            preferences: {
              messages: true,
              agents: true,
              skills: true,
              system: true,
              meetings: true,
              reminders: true,
              important: true,
            },
            integrationItems: [],
            integrationUnreadCount: 0,
            integrationLoading: false,
            integrationError: null,
            ...preloaded,
          },
        }
      : undefined,
  });
}

async function importPanel() {
  vi.resetModules();
  const mod = await import('./NotificationsPanel');
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

describe('<NotificationsPanel />', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    getBypassPrefsMock.mockResolvedValue({ global_dnd: false });
    setGlobalDndMock.mockResolvedValue(undefined);
  });

  it('renders one toggle per category plus the DND switch', async () => {
    const Panel = await importPanel();
    renderPanel(Panel);

    // The DND toggle starts in a loading skeleton state until getBypassPrefs
    // resolves; the seven category toggles render synchronously.
    await waitFor(() => {
      expect(screen.getByLabelText('Toggle Do Not Disturb')).toBeInTheDocument();
    });
    expect(screen.getByLabelText('Toggle Messages notifications')).toBeInTheDocument();
    expect(screen.getByLabelText('Toggle Agent activity notifications')).toBeInTheDocument();
    expect(screen.getByLabelText('Toggle Skills notifications')).toBeInTheDocument();
    expect(screen.getByLabelText('Toggle System notifications')).toBeInTheDocument();
    expect(screen.getByLabelText('Toggle Meetings notifications')).toBeInTheDocument();
    expect(screen.getByLabelText('Toggle Reminders notifications')).toBeInTheDocument();
    expect(screen.getByLabelText('Toggle Important events notifications')).toBeInTheDocument();
  });

  it('reflects the persisted global_dnd value once getBypassPrefs resolves', async () => {
    getBypassPrefsMock.mockResolvedValueOnce({ global_dnd: true });
    const Panel = await importPanel();
    renderPanel(Panel);

    await waitFor(() => {
      const dnd = screen.getByLabelText('Toggle Do Not Disturb');
      expect(dnd).toHaveAttribute('aria-checked', 'true');
    });
  });

  it('toggling DND calls setGlobalDnd and updates the optimistic UI', async () => {
    const Panel = await importPanel();
    renderPanel(Panel);

    const dnd = await screen.findByLabelText('Toggle Do Not Disturb');
    expect(dnd).toHaveAttribute('aria-checked', 'false');

    fireEvent.click(dnd);
    await waitFor(() => expect(setGlobalDndMock).toHaveBeenCalledWith(true));
    expect(dnd).toHaveAttribute('aria-checked', 'true');
  });

  it('rolls back the DND toggle when setGlobalDnd rejects', async () => {
    setGlobalDndMock.mockRejectedValueOnce(new Error('rpc down'));
    const Panel = await importPanel();
    renderPanel(Panel);

    const dnd = await screen.findByLabelText('Toggle Do Not Disturb');
    fireEvent.click(dnd);
    // After the rejection, the optimistic flip is undone.
    await waitFor(() => expect(setGlobalDndMock).toHaveBeenCalled());
    await waitFor(() => expect(dnd).toHaveAttribute('aria-checked', 'false'));
  });

  it('clicking a category toggle flips its aria-checked state', async () => {
    const Panel = await importPanel();
    renderPanel(Panel);

    const messagesToggle = screen.getByLabelText('Toggle Messages notifications');
    // Default preference is `true`; clicking should flip to `false`.
    expect(messagesToggle).toHaveAttribute('aria-checked', 'true');
    fireEvent.click(messagesToggle);
    expect(messagesToggle).toHaveAttribute('aria-checked', 'false');
  });

  it('reads category state from the redux preferences slice', async () => {
    const Panel = await importPanel();
    const store = buildStore({
      preferences: {
        messages: false,
        agents: true,
        skills: false,
        system: true,
        meetings: true,
        reminders: false,
        important: true,
      },
    });
    renderPanel(Panel, store);

    expect(screen.getByLabelText('Toggle Messages notifications')).toHaveAttribute(
      'aria-checked',
      'false'
    );
    expect(screen.getByLabelText('Toggle Skills notifications')).toHaveAttribute(
      'aria-checked',
      'false'
    );
    expect(screen.getByLabelText('Toggle Agent activity notifications')).toHaveAttribute(
      'aria-checked',
      'true'
    );
  });
});
