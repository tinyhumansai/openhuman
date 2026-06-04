/**
 * Tests for the SecurityBanner.
 *
 * Confirms the host-aware approval-gate boot state surfaces the right banner:
 * - `disabledByEnv` renders the persistent red banner.
 * - `overrideIgnored` renders the one-shot yellow info banner that auto-dismisses.
 * - Steady-state (installed, no override) renders nothing.
 */
import { configureStore } from '@reduxjs/toolkit';
import { act, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { I18nProvider } from '../../lib/i18n/I18nContext';
import type { ApprovalGateBootState } from '../../services/api/approvalApi';
import localeReducer from '../../store/localeSlice';
import SecurityBanner from '../SecurityBanner';

function renderBanner(state: ApprovalGateBootState | Promise<ApprovalGateBootState>) {
  const fetchState = vi
    .fn()
    .mockReturnValue(state instanceof Promise ? state : Promise.resolve(state));
  const store = configureStore({
    reducer: { locale: localeReducer },
    preloadedState: { locale: { current: 'en' as const } },
  });
  return {
    fetchState,
    ...render(
      <Provider store={store}>
        <I18nProvider>
          <SecurityBanner fetchState={fetchState} />
        </I18nProvider>
      </Provider>
    ),
  };
}

describe('SecurityBanner', () => {
  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.clearAllMocks();
  });

  it('renders nothing on the steady-state desktop boot path', async () => {
    const { fetchState } = renderBanner({
      installed: true,
      disabledByEnv: false,
      overrideIgnored: false,
      host: 'tauri-shell',
    });

    await waitFor(() => expect(fetchState).toHaveBeenCalledTimes(1));

    expect(screen.queryByTestId('security-banner-gate-disabled')).toBeNull();
    expect(screen.queryByTestId('security-banner-override-ignored')).toBeNull();
  });

  it('renders the persistent red banner when the gate is disabled by env override', async () => {
    renderBanner({ installed: false, disabledByEnv: true, overrideIgnored: false, host: 'cli' });

    const banner = await screen.findByTestId('security-banner-gate-disabled');
    expect(banner).toBeInTheDocument();
    expect(banner.getAttribute('role')).toBe('alert');
  });

  it('renders the one-shot info banner when the env override was ignored under desktop shell', async () => {
    renderBanner({
      installed: true,
      disabledByEnv: false,
      overrideIgnored: true,
      host: 'tauri-shell',
    });

    const banner = await screen.findByTestId('security-banner-override-ignored');
    expect(banner).toBeInTheDocument();
    expect(banner.getAttribute('role')).toBe('status');
  });

  it('auto-dismisses the override-ignored info banner after 10 seconds', async () => {
    renderBanner({
      installed: true,
      disabledByEnv: false,
      overrideIgnored: true,
      host: 'tauri-shell',
    });

    await screen.findByTestId('security-banner-override-ignored');

    await act(async () => {
      vi.advanceTimersByTime(10_000);
    });

    expect(screen.queryByTestId('security-banner-override-ignored')).toBeNull();
  });

  it('does NOT auto-dismiss the persistent disabled banner', async () => {
    renderBanner({ installed: false, disabledByEnv: true, overrideIgnored: false, host: 'cli' });

    await screen.findByTestId('security-banner-gate-disabled');

    await act(async () => {
      vi.advanceTimersByTime(60_000);
    });

    expect(screen.queryByTestId('security-banner-gate-disabled')).toBeInTheDocument();
  });

  it('renders nothing on RPC failure (degraded core must not blank the app shell)', async () => {
    const { fetchState } = renderBanner(Promise.reject(new Error('rpc failed')));

    await waitFor(() => expect(fetchState).toHaveBeenCalledTimes(1));

    expect(screen.queryByTestId('security-banner-gate-disabled')).toBeNull();
    expect(screen.queryByTestId('security-banner-override-ignored')).toBeNull();
  });
});
