/**
 * Tests for the global app-update prompt.
 *
 * Drives the underlying `useAppUpdate` hook through the shared mocks and
 * asserts the user-visible UX contract:
 *   - silent during background download (no banner on `available`/`downloading`)
 *   - prompt with "Restart now" / "Later" once bytes are staged
 *     (`ready_to_install`)
 *   - error surface with retry path
 */
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import AppUpdatePrompt from '../AppUpdatePrompt';

const hoisted = vi.hoisted(() => ({
  mockCheckAppUpdate: vi.fn(),
  mockApplyAppUpdate: vi.fn(),
  mockDownloadAppUpdate: vi.fn(),
  mockInstallAppUpdate: vi.fn(),
  mockIsTauri: vi.fn(() => true),
  statusListeners: [] as ((event: { payload: string }) => void)[],
}));

const {
  mockCheckAppUpdate,
  mockApplyAppUpdate,
  mockDownloadAppUpdate,
  mockInstallAppUpdate,
  mockIsTauri,
  statusListeners,
} = hoisted;

vi.mock('../../utils/tauriCommands', () => ({
  checkAppUpdate: hoisted.mockCheckAppUpdate,
  applyAppUpdate: hoisted.mockApplyAppUpdate,
  downloadAppUpdate: hoisted.mockDownloadAppUpdate,
  installAppUpdate: hoisted.mockInstallAppUpdate,
  isTauri: hoisted.mockIsTauri,
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((event: string, handler: (event: { payload: string }) => void) => {
    if (event === 'app-update:status') {
      hoisted.statusListeners.push(handler);
    }
    return Promise.resolve(() => {
      const idx = hoisted.statusListeners.indexOf(handler);
      if (idx >= 0) hoisted.statusListeners.splice(idx, 1);
    });
  }),
}));

const emitStatus = (payload: string) => {
  for (const listener of [...statusListeners]) listener({ payload });
};

describe('AppUpdatePrompt', () => {
  beforeEach(() => {
    statusListeners.length = 0;
    mockCheckAppUpdate.mockReset();
    mockApplyAppUpdate.mockReset();
    mockDownloadAppUpdate.mockReset();
    mockInstallAppUpdate.mockReset();
    mockIsTauri.mockReturnValue(true);
  });

  it('stays silent while a download is in progress', async () => {
    mockCheckAppUpdate.mockResolvedValueOnce({
      current_version: '0.50.0',
      available: true,
      available_version: '0.51.0',
      body: null,
    });
    // Simulate a check that finds an update + a download that's still
    // running — the hook will move into "available" then "downloading".
    mockDownloadAppUpdate.mockImplementation(
      () =>
        new Promise(() => {
          /* never resolves during the test */
        })
    );

    renderWithProviders(<AppUpdatePrompt initialCheckDelayMs={0} recheckIntervalMs={0} />);

    // Give the auto-check + auto-download timers a chance to run.
    await new Promise(resolve => setTimeout(resolve, 50));

    expect(screen.queryByTestId('app-update-prompt')).not.toBeInTheDocument();
  });

  it('shows the "Restart now" prompt once the download is staged', async () => {
    renderWithProviders(
      <AppUpdatePrompt autoCheck={false} initialCheckDelayMs={0} recheckIntervalMs={0} />
    );
    // Wait for listeners to register.
    await waitFor(() => expect(statusListeners.length).toBeGreaterThan(0));

    // Simulate the Rust side emitting ready_to_install.
    emitStatus('ready_to_install');

    await waitFor(() => {
      expect(screen.getByText('Update ready to install')).toBeInTheDocument();
    });
    expect(screen.getByRole('button', { name: /Restart now/ })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Later/ })).toBeInTheDocument();
  });

  it('clicking "Restart now" invokes installAppUpdate (the staged path)', async () => {
    mockInstallAppUpdate.mockResolvedValueOnce(undefined);

    renderWithProviders(
      <AppUpdatePrompt autoCheck={false} initialCheckDelayMs={0} recheckIntervalMs={0} />
    );
    await waitFor(() => expect(statusListeners.length).toBeGreaterThan(0));

    // The Rust side emits `ready_to_install` once bytes are staged. The
    // hook's status listener flips `stagedRef` to true on that event, so a
    // subsequent install() must take the fast staged path and call
    // `installAppUpdate` directly — never falling back to the legacy
    // combined `applyAppUpdate`.
    emitStatus('ready_to_install');

    const restartBtn = await screen.findByRole('button', { name: /Restart now/ });
    fireEvent.click(restartBtn);

    await waitFor(() => {
      expect(mockInstallAppUpdate).toHaveBeenCalledTimes(1);
    });
    expect(mockApplyAppUpdate).not.toHaveBeenCalled();
  });

  it('clicking "Later" hides the banner without calling install', async () => {
    renderWithProviders(
      <AppUpdatePrompt autoCheck={false} initialCheckDelayMs={0} recheckIntervalMs={0} />
    );
    await waitFor(() => expect(statusListeners.length).toBeGreaterThan(0));

    emitStatus('ready_to_install');

    const laterBtn = await screen.findByRole('button', { name: /Later/ });
    fireEvent.click(laterBtn);

    await waitFor(() => {
      expect(screen.queryByText('Update ready to install')).not.toBeInTheDocument();
    });
    expect(mockInstallAppUpdate).not.toHaveBeenCalled();
    expect(mockApplyAppUpdate).not.toHaveBeenCalled();
  });

  it('renders an error banner with retry on failure', async () => {
    renderWithProviders(
      <AppUpdatePrompt autoCheck={false} initialCheckDelayMs={0} recheckIntervalMs={0} />
    );
    await waitFor(() => expect(statusListeners.length).toBeGreaterThan(0));

    emitStatus('error');

    await waitFor(() => {
      expect(screen.getByText('Update failed')).toBeInTheDocument();
    });
    expect(screen.getByRole('button', { name: /Try again/ })).toBeInTheDocument();
  });

  it('clicking "Try again" after error invokes downloadAppUpdate', async () => {
    mockDownloadAppUpdate.mockResolvedValueOnce({ ready: true, version: '0.51.0', body: null });

    renderWithProviders(
      <AppUpdatePrompt autoCheck={false} initialCheckDelayMs={0} recheckIntervalMs={0} />
    );
    await waitFor(() => expect(statusListeners.length).toBeGreaterThan(0));

    emitStatus('error');

    const retryBtn = await screen.findByRole('button', { name: /Try again/ });
    fireEvent.click(retryBtn);

    await waitFor(() => {
      expect(mockDownloadAppUpdate).toHaveBeenCalledTimes(1);
    });
  });

  it('shows the installing-phase banner with progress copy', async () => {
    renderWithProviders(
      <AppUpdatePrompt autoCheck={false} initialCheckDelayMs={0} recheckIntervalMs={0} />
    );
    await waitFor(() => expect(statusListeners.length).toBeGreaterThan(0));

    emitStatus('installing');

    await waitFor(() => {
      expect(screen.getByText('Installing update')).toBeInTheDocument();
    });
    expect(screen.getByText(/Installing the new version/i)).toBeInTheDocument();
  });

  it('shows the restarting-phase banner', async () => {
    renderWithProviders(
      <AppUpdatePrompt autoCheck={false} initialCheckDelayMs={0} recheckIntervalMs={0} />
    );
    await waitFor(() => expect(statusListeners.length).toBeGreaterThan(0));

    emitStatus('restarting');

    await waitFor(() => {
      // Header label is "Restarting…" (with the ellipsis char).
      expect(screen.getByText(/Restarting/)).toBeInTheDocument();
    });
  });

  it('clicking "Dismiss" on the error banner hides the prompt', async () => {
    renderWithProviders(
      <AppUpdatePrompt autoCheck={false} initialCheckDelayMs={0} recheckIntervalMs={0} />
    );
    await waitFor(() => expect(statusListeners.length).toBeGreaterThan(0));

    emitStatus('error');
    await waitFor(() => expect(screen.getByText('Update failed')).toBeInTheDocument());

    fireEvent.click(screen.getByRole('button', { name: /^Dismiss$/i }));

    await waitFor(() => {
      expect(screen.queryByText('Update failed')).not.toBeInTheDocument();
    });
  });

  it('renders nothing when not in Tauri', async () => {
    mockIsTauri.mockReturnValue(false);

    renderWithProviders(<AppUpdatePrompt initialCheckDelayMs={0} recheckIntervalMs={0} />);

    await new Promise(resolve => setTimeout(resolve, 30));
    expect(screen.queryByTestId('app-update-prompt')).not.toBeInTheDocument();
  });
});
