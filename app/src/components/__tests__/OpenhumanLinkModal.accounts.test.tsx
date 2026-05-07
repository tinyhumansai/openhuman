import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import accountsReducer from '../../store/accountsSlice';
import OpenhumanLinkModal, { OPENHUMAN_LINK_EVENT } from '../OpenhumanLinkModal';

// Mock modules that require Tauri runtime
vi.mock('@tauri-apps/api/core', () => ({ isTauri: vi.fn(() => false) }));
vi.mock('../../lib/nativeNotifications/tauriBridge', () => ({
  ensureNotificationPermission: vi.fn(),
  getNotificationPermissionState: vi.fn().mockResolvedValue('prompt'),
  showNativeNotification: vi.fn(),
}));
vi.mock('../../services/webviewAccountService', () => ({
  isTauri: vi.fn(() => false),
  purgeWebviewAccount: vi.fn().mockResolvedValue(undefined),
}));

const mockNavigate = vi.fn();
vi.mock('react-router-dom', async () => {
  const actual = await vi.importActual('react-router-dom');
  return { ...actual, useNavigate: () => mockNavigate };
});

function createStore() {
  return configureStore({
    reducer: combineReducers({
      accounts: accountsReducer,
      // Stubs for selectors that may be read elsewhere
      channelConnections: () => ({}),
    }),
  });
}

function renderModal(store = createStore()) {
  return {
    store,
    ...render(
      <Provider store={store}>
        <MemoryRouter>
          <OpenhumanLinkModal />
        </MemoryRouter>
      </Provider>
    ),
  };
}

function openAccountsModal() {
  act(() => {
    window.dispatchEvent(
      new CustomEvent(OPENHUMAN_LINK_EVENT, { detail: { path: 'accounts/setup' } })
    );
  });
}

describe('OpenhumanLinkModal accounts setup', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders provider toggles when accounts/setup path is opened', () => {
    renderModal();
    openAccountsModal();

    expect(screen.getByLabelText('Connect WhatsApp Web')).toBeInTheDocument();
    expect(screen.getByLabelText('Connect Telegram Web')).toBeInTheDocument();
    expect(screen.getByLabelText('Connect Slack')).toBeInTheDocument();
    expect(screen.getByLabelText('Connect Discord')).toBeInTheDocument();
    expect(screen.getByLabelText('Connect LinkedIn')).toBeInTheDocument();
  });

  it('toggle ON adds account to Redux store', () => {
    const { store } = renderModal();
    openAccountsModal();

    fireEvent.click(screen.getByLabelText('Connect Telegram Web'));

    const state = store.getState().accounts;
    const telegramAccount = Object.values(state.accounts).find(a => a.provider === 'telegram');
    expect(telegramAccount).toBeDefined();
    expect(telegramAccount!.status).toBe('pending');
  });

  it('toggle OFF removes account from Redux store', () => {
    const { store } = renderModal();
    openAccountsModal();

    // Toggle ON
    fireEvent.click(screen.getByLabelText('Connect Telegram Web'));
    expect(Object.values(store.getState().accounts.accounts)).toHaveLength(1);

    // Toggle OFF
    fireEvent.click(screen.getByLabelText('Disconnect Telegram Web'));
    expect(Object.values(store.getState().accounts.accounts)).toHaveLength(0);
  });

  it('Done button navigates to /chat and sets first new account as active', () => {
    const { store } = renderModal();
    openAccountsModal();

    // Toggle two providers ON
    fireEvent.click(screen.getByLabelText('Connect Telegram Web'));
    fireEvent.click(screen.getByLabelText('Connect Slack'));

    const accountIds = store.getState().accounts.order;
    expect(accountIds).toHaveLength(2);

    // Click the CTA (dynamic label: "Continue with Telegram Web sign-in")
    fireEvent.click(screen.getByRole('button', { name: /Continue with Telegram Web sign-in/ }));

    expect(store.getState().accounts.activeAccountId).toBe(accountIds[0]);
    expect(mockNavigate).toHaveBeenCalledWith('/chat');
  });

  it('Skip button closes modal without navigating', () => {
    renderModal();
    openAccountsModal();

    fireEvent.click(screen.getByLabelText('Connect Telegram Web'));
    fireEvent.click(screen.getByRole('button', { name: 'Skip for now' }));

    expect(mockNavigate).not.toHaveBeenCalled();
  });

  it('Done without any new toggles does not navigate', () => {
    renderModal();
    openAccountsModal();

    fireEvent.click(screen.getByRole('button', { name: 'Done' }));
    expect(mockNavigate).not.toHaveBeenCalled();
  });

  it('shows dynamic CTA label when a provider is toggled on', () => {
    renderModal();
    openAccountsModal();

    // Before toggling, button says "Done"
    expect(screen.getByRole('button', { name: 'Done' })).toBeInTheDocument();

    // Toggle Discord on
    fireEvent.click(screen.getByLabelText('Connect Discord'));

    // CTA should now reference Discord
    expect(
      screen.getByRole('button', { name: /Continue with Discord sign-in/ })
    ).toBeInTheDocument();
  });

  it('shows status indicator for existing accounts with a status', () => {
    const store = createStore();
    // Pre-populate an account with 'open' status
    store.dispatch({
      type: 'accounts/addAccount',
      payload: {
        id: 'test-acct-1',
        provider: 'telegram',
        label: 'Telegram',
        createdAt: new Date().toISOString(),
        status: 'open',
      },
    });

    render(
      <Provider store={store}>
        <MemoryRouter>
          <OpenhumanLinkModal />
        </MemoryRouter>
      </Provider>
    );
    openAccountsModal();

    expect(screen.getByText('Connected')).toBeInTheDocument();
  });

  it('shows "Needs sign-in" for accounts with pending status', () => {
    const store = createStore();
    store.dispatch({
      type: 'accounts/addAccount',
      payload: {
        id: 'test-acct-2',
        provider: 'slack',
        label: 'Slack',
        createdAt: new Date().toISOString(),
        status: 'pending',
      },
    });

    render(
      <Provider store={store}>
        <MemoryRouter>
          <OpenhumanLinkModal />
        </MemoryRouter>
      </Provider>
    );
    openAccountsModal();

    expect(screen.getByText('Needs sign-in')).toBeInTheDocument();
  });
});
