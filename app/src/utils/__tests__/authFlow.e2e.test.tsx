import { isTauri as tauriRuntimeIsTauri } from '@tauri-apps/api/core';
import { getCurrent, onOpenUrl } from '@tauri-apps/plugin-deep-link';
import { render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { beforeEach, describe, expect, it, type Mock, vi } from 'vitest';

import * as tauriCommands from '../tauriCommands';
// @ts-ignore - test-only JS module outside app/src
import { clearRequestLog, getRequestLog } from '../../../../scripts/mock-api-core.mjs';
import PublicRoute from '../../components/PublicRoute';
import UserProvider from '../../providers/UserProvider';
import { callCoreRpc } from '../../services/coreRpcClient';
import { store } from '../../store';
import { clearToken, setAuthBootstrapComplete } from '../../store/authSlice';
import { setupDesktopDeepLinkListener } from '../desktopDeepLinkListener';

vi.mock('../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

describe('Auth flow e2e (binary + OAuth callback)', () => {
  const mockIsTauriRuntime = tauriRuntimeIsTauri as Mock;
  const mockGetCurrent = getCurrent as Mock;
  const mockOnOpenUrl = onOpenUrl as Mock;
  const mockCallCoreRpc = callCoreRpc as Mock;

  const mockIsTauriCommand = tauriCommands.isTauri as Mock;
  const mockGetAuthState = tauriCommands.getAuthState as Mock;
  const mockGetSessionToken = tauriCommands.getSessionToken as Mock;
  const mockStoreSession = tauriCommands.storeSession as Mock;
  const mockSyncMemoryClientToken = tauriCommands.syncMemoryClientToken as Mock;
  const mockLogout = tauriCommands.logout as Mock;

  beforeEach(async () => {
    vi.clearAllMocks();
    clearRequestLog();
    await store.dispatch(clearToken());
    store.dispatch(setAuthBootstrapComplete(false));
    window.location.hash = '/';

    mockIsTauriRuntime.mockReturnValue(false);
    mockGetCurrent.mockResolvedValue(null);
    mockOnOpenUrl.mockResolvedValue(() => {});

    mockIsTauriCommand.mockReturnValue(false);
    mockGetAuthState.mockResolvedValue({ is_authenticated: false, user: null });
    mockGetSessionToken.mockResolvedValue(null);
    mockStoreSession.mockResolvedValue(undefined);
    mockSyncMemoryClientToken.mockResolvedValue(undefined);
    mockLogout.mockResolvedValue(undefined);
  });

  it('bootstraps token from core auth state and routes public entry to /home', async () => {
    mockIsTauriCommand.mockReturnValue(true);
    mockGetAuthState.mockResolvedValue({ is_authenticated: true, user: { _id: 'user-123' } });
    mockGetSessionToken.mockResolvedValue('jwt-from-core');

    render(
      <Provider store={store}>
        <MemoryRouter initialEntries={['/']}>
          <UserProvider>
            <Routes>
              <Route
                path="/"
                element={
                  <PublicRoute>
                    <div>Welcome</div>
                  </PublicRoute>
                }
              />
              <Route path="/home" element={<div>Home</div>} />
            </Routes>
          </UserProvider>
        </MemoryRouter>
      </Provider>
    );

    await waitFor(() => expect(screen.getByText('Home')).toBeInTheDocument());
    expect(store.getState().auth.token).toBe('jwt-from-core');
    expect(mockGetAuthState).toHaveBeenCalledTimes(1);
    expect(mockGetSessionToken).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(mockStoreSession).toHaveBeenCalledWith('jwt-from-core', { id: '' }));

    await waitFor(() => {
      const requests = getRequestLog() as Array<{ method: string; url: string }>;
      expect(requests.some(req => req.method === 'GET' && req.url.startsWith('/telegram/me'))).toBe(
        true
      );
    });
  });

  it('consumes OAuth login token from deep link and updates auth token + redirect', async () => {
    mockIsTauriRuntime.mockReturnValue(true);
    mockCallCoreRpc.mockResolvedValue({ result: { jwtToken: 'jwt-from-login-token' }, logs: [] });

    await setupDesktopDeepLinkListener();
    expect(mockOnOpenUrl).toHaveBeenCalledTimes(1);

    const openUrlHandler = mockOnOpenUrl.mock.calls[0][0] as (urls: string[]) => void;
    openUrlHandler(['openhuman://auth?token=oauth-login-token-123']);

    await waitFor(() =>
      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.auth.consume_login_token',
        params: { loginToken: 'oauth-login-token-123' },
      })
    );
    await waitFor(() => expect(store.getState().auth.token).toBe('jwt-from-login-token'));
    expect(window.location.hash).toBe('#/home');
    await waitFor(() =>
      expect(mockStoreSession).toHaveBeenCalledWith('jwt-from-login-token', { id: '' })
    );
  });
});
