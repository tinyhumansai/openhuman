/**
 * Tests for CoreConnectionPanel (GH-4396) — the first-class Settings surface
 * that promotes cloud-mode remote-core config and adds a live status
 * indicator. Covers: live status rendering per mode, the remote toggle
 * revealing the URL/token form, and the save flow persisting + dispatching +
 * restarting.
 */
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';

const hoisted = vi.hoisted(() => ({
  testCoreRpcConnection: vi.fn(),
  clearCoreRpcUrlCache: vi.fn(),
  clearCoreRpcTokenCache: vi.fn(),
  restartApp: vi.fn(),
  // Default to a desktop (Tauri) context so local mode is available; the
  // web-build test flips this to false to assert the toggle is gated off.
  isTauriEnvironment: vi.fn(() => true),
  invoke: vi.fn(async () => 'http://127.0.0.1:7788/rpc'),
}));

vi.mock('../../../../services/coreRpcClient', () => ({
  testCoreRpcConnection: hoisted.testCoreRpcConnection,
  clearCoreRpcUrlCache: hoisted.clearCoreRpcUrlCache,
  clearCoreRpcTokenCache: hoisted.clearCoreRpcTokenCache,
}));

vi.mock('../../../../utils/tauriCommands/core', () => ({ restartApp: hoisted.restartApp }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: hoisted.invoke }));

// Keep the real configPersistence (storeCoreMode/localStorage etc.) but control
// isTauriEnvironment so the desktop-vs-web-build gating is testable.
vi.mock('../../../../utils/configPersistence', async importOriginal => {
  const actual = await importOriginal<typeof import('../../../../utils/configPersistence')>();
  return { ...actual, isTauriEnvironment: hoisted.isTauriEnvironment };
});

function okResponse() {
  return { ok: true, status: 200, json: async () => ({ jsonrpc: '2.0', id: 1, result: {} }) };
}

/** A Response-shaped stub for an arbitrary HTTP status. */
function statusResponse(status: number) {
  return { ok: status >= 200 && status < 300, status, json: async () => ({}) };
}

const CLOUD_STATE = {
  coreMode: { mode: { kind: 'cloud', url: 'https://core.example.com/rpc', token: 'tok-123456' } },
};

async function importPanel() {
  const mod = await import('../CoreConnectionPanel');
  return mod.default;
}

describe('CoreConnectionPanel', () => {
  beforeEach(() => {
    vi.resetModules();
    hoisted.testCoreRpcConnection.mockReset();
    hoisted.clearCoreRpcUrlCache.mockReset();
    hoisted.clearCoreRpcTokenCache.mockReset();
    hoisted.restartApp.mockReset();
    hoisted.restartApp.mockResolvedValue(undefined);
    hoisted.isTauriEnvironment.mockReset();
    hoisted.isTauriEnvironment.mockReturnValue(true);
    hoisted.invoke.mockReset();
    hoisted.invoke.mockResolvedValue('http://127.0.0.1:7788/rpc');
    localStorage.clear();
  });

  test('local mode shows the local connected status once the live check passes', async () => {
    hoisted.testCoreRpcConnection.mockResolvedValue(okResponse());
    const Panel = await importPanel();
    renderWithProviders(<Panel />, { preloadedState: { coreMode: { mode: { kind: 'local' } } } });

    await waitFor(() => expect(screen.getByText('Connected to local core')).toBeInTheDocument());
    // Remote toggle is off in local mode → no URL field.
    expect(screen.queryByLabelText(/Runtime URL/i)).not.toBeInTheDocument();
  });

  test('cloud mode surfaces the remote URL and remote connected status', async () => {
    hoisted.testCoreRpcConnection.mockResolvedValue(okResponse());
    const Panel = await importPanel();
    renderWithProviders(<Panel />, {
      preloadedState: {
        coreMode: {
          mode: { kind: 'cloud', url: 'https://core.example.com/rpc', token: 'tok-123456' },
        },
      },
    });

    await waitFor(() => expect(screen.getByText('Connected to remote core')).toBeInTheDocument());
    // Toggle on → the URL field is pre-filled with the persisted value.
    expect(screen.getByDisplayValue('https://core.example.com/rpc')).toBeInTheDocument();
  });

  test('unreachable core surfaces the failure status', async () => {
    hoisted.testCoreRpcConnection.mockRejectedValue(new Error('boom'));
    const Panel = await importPanel();
    renderWithProviders(<Panel />, { preloadedState: { coreMode: { mode: { kind: 'local' } } } });

    await waitFor(() => expect(screen.getByText(/Cannot reach the core/i)).toBeInTheDocument());
  });

  test('switching to remote core persists, dispatches, and restarts', async () => {
    hoisted.testCoreRpcConnection.mockResolvedValue(okResponse());
    const Panel = await importPanel();
    const { store } = renderWithProviders(<Panel />, {
      preloadedState: { coreMode: { mode: { kind: 'local' } } },
    });

    await waitFor(() => expect(screen.getByText('Connected to local core')).toBeInTheDocument());

    // Flip the remote toggle on to reveal the form.
    fireEvent.click(screen.getByTestId('core-use-remote-toggle'));

    fireEvent.change(screen.getByLabelText(/Runtime URL/i), {
      target: { value: 'https://core.example.com/rpc' },
    });
    fireEvent.change(screen.getByLabelText(/Auth Token/i), {
      target: { value: 'remote-token-xyz' },
    });

    fireEvent.click(screen.getByTestId('core-save-btn'));

    await waitFor(() => expect(hoisted.restartApp).toHaveBeenCalledTimes(1));

    // Redux is now in cloud mode with the typed URL + token.
    const mode = store.getState().coreMode.mode as { kind: string; url?: string; token?: string };
    expect(mode.kind).toBe('cloud');
    expect(mode.url).toBe('https://core.example.com/rpc');
    expect(mode.token).toBe('remote-token-xyz');

    // Persisted synchronously to localStorage (mirrors the cloud-mode picker).
    expect(localStorage.getItem('openhuman_core_mode')).toBe('cloud');
    expect(localStorage.getItem('openhuman_core_rpc_url')).toBe('https://core.example.com/rpc');
    expect(localStorage.getItem('openhuman_core_rpc_token')).toBe('remote-token-xyz');

    // Caches cleared so the new endpoint takes effect on restart.
    expect(hoisted.clearCoreRpcUrlCache).toHaveBeenCalled();
    expect(hoisted.clearCoreRpcTokenCache).toHaveBeenCalled();
  });

  test('a rejected token surfaces the token-rejected live status', async () => {
    hoisted.testCoreRpcConnection.mockResolvedValue(statusResponse(401));
    const Panel = await importPanel();
    renderWithProviders(<Panel />, { preloadedState: CLOUD_STATE });

    await waitFor(() => expect(screen.getByText(/the token was rejected/i)).toBeInTheDocument());
  });

  test('a non-ok response surfaces the unreachable live status with the HTTP code', async () => {
    hoisted.testCoreRpcConnection.mockResolvedValue(statusResponse(503));
    const Panel = await importPanel();
    renderWithProviders(<Panel />, { preloadedState: CLOUD_STATE });

    await waitFor(() =>
      expect(screen.getByText(/Cannot reach the core — HTTP 503/i)).toBeInTheDocument()
    );
  });

  test('Test connection reports success for the typed remote inputs', async () => {
    hoisted.testCoreRpcConnection.mockResolvedValue(okResponse());
    const Panel = await importPanel();
    renderWithProviders(<Panel />, { preloadedState: CLOUD_STATE });

    // Wait for the mount live-check to settle first.
    await waitFor(() => expect(screen.getByText('Connected to remote core')).toBeInTheDocument());

    fireEvent.click(screen.getByText('Test Connection'));
    await waitFor(() => expect(screen.getByTestId('core-test-ok')).toBeInTheDocument());
  });

  test('Test connection reports an auth failure', async () => {
    hoisted.testCoreRpcConnection.mockResolvedValue(statusResponse(403));
    const Panel = await importPanel();
    renderWithProviders(<Panel />, { preloadedState: CLOUD_STATE });

    fireEvent.click(screen.getByText('Test Connection'));
    await waitFor(() => expect(screen.getByTestId('core-test-auth')).toBeInTheDocument());
  });

  test('Test connection reports an unreachable endpoint', async () => {
    hoisted.testCoreRpcConnection.mockRejectedValue(new Error('network down'));
    const Panel = await importPanel();
    renderWithProviders(<Panel />, { preloadedState: CLOUD_STATE });

    fireEvent.click(screen.getByText('Test Connection'));
    await waitFor(() => expect(screen.getByTestId('core-test-unreachable')).toBeInTheDocument());
  });

  test('validation blocks the form when the URL is empty and when the token is missing', async () => {
    hoisted.testCoreRpcConnection.mockResolvedValue(okResponse());
    const Panel = await importPanel();
    renderWithProviders(<Panel />, { preloadedState: { coreMode: { mode: { kind: 'local' } } } });

    await waitFor(() => expect(screen.getByText('Connected to local core')).toBeInTheDocument());
    // Reveal the empty remote form.
    fireEvent.click(screen.getByTestId('core-use-remote-toggle'));

    // Empty URL → invalid-URL error.
    fireEvent.click(screen.getByText('Test Connection'));
    await waitFor(() => expect(screen.getByText(/enter a runtime URL/i)).toBeInTheDocument());

    // Valid URL but empty token → token-required error.
    fireEvent.change(screen.getByLabelText(/Runtime URL/i), {
      target: { value: 'https://core.example.com/rpc' },
    });
    fireEvent.click(screen.getByText('Test Connection'));
    await waitFor(() =>
      expect(screen.getByText(/need an auth token to connect/i)).toBeInTheDocument()
    );

    // The typed connection was never attempted (validation short-circuits).
    expect(hoisted.testCoreRpcConnection).not.toHaveBeenCalledWith(
      'https://core.example.com/rpc',
      ''
    );
  });

  test('web build (non-Tauri) forces remote and disables the local toggle', async () => {
    hoisted.isTauriEnvironment.mockReturnValue(false);
    hoisted.testCoreRpcConnection.mockResolvedValue(okResponse());
    const Panel = await importPanel();
    renderWithProviders(<Panel />, {
      preloadedState: {
        coreMode: {
          mode: { kind: 'cloud', url: 'https://core.example.com/rpc', token: 'tok-123456' },
        },
      },
    });

    await waitFor(() => expect(screen.getByText('Connected to remote core')).toBeInTheDocument());
    // A browser can't start a local core, so the toggle is forced on + disabled.
    const toggle = screen.getByTestId('core-use-remote-toggle');
    expect(toggle).toBeDisabled();
    expect(screen.getByDisplayValue('https://core.example.com/rpc')).toBeInTheDocument();
  });

  test('switching from remote back to local clears persistence, dispatches, and restarts', async () => {
    hoisted.testCoreRpcConnection.mockResolvedValue(okResponse());
    // Seed persisted cloud values so we can assert they are cleared.
    localStorage.setItem('openhuman_core_mode', 'cloud');
    localStorage.setItem('openhuman_core_rpc_url', 'https://core.example.com/rpc');
    localStorage.setItem('openhuman_core_rpc_token', 'tok-123456');

    const Panel = await importPanel();
    const { store } = renderWithProviders(<Panel />, { preloadedState: CLOUD_STATE });

    await waitFor(() => expect(screen.getByText('Connected to remote core')).toBeInTheDocument());

    // Flip remote off → local, then save.
    fireEvent.click(screen.getByTestId('core-use-remote-toggle'));
    fireEvent.click(screen.getByTestId('core-save-btn'));

    await waitFor(() => expect(hoisted.restartApp).toHaveBeenCalledTimes(1));

    expect(store.getState().coreMode.mode.kind).toBe('local');
    expect(localStorage.getItem('openhuman_core_mode')).toBe('local');
    expect(localStorage.getItem('openhuman_core_rpc_url')).toBeNull();
    expect(localStorage.getItem('openhuman_core_rpc_token')).toBeNull();
  });
});
