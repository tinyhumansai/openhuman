/**
 * Tests for the Settings → About panel.
 *
 * Covers the basic render (version + summary copy), the manual
 * "Check for updates" button (invoking the hook's `check`), and the
 * summary text variants for the new download/install state machine.
 */
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import AboutPanel from '../AboutPanel';

const hoisted = vi.hoisted(() => ({
  mockCheckAppUpdate: vi.fn(),
  mockApplyAppUpdate: vi.fn(),
  mockDownloadAppUpdate: vi.fn(),
  mockInstallAppUpdate: vi.fn(),
  mockIsTauri: vi.fn(() => true),
  mockOpenUrl: vi.fn(),
  statusListeners: [] as ((event: { payload: string }) => void)[],
}));

const { mockCheckAppUpdate, mockOpenUrl, statusListeners } = hoisted;

vi.mock('../../../../utils/tauriCommands', () => ({
  checkAppUpdate: hoisted.mockCheckAppUpdate,
  applyAppUpdate: hoisted.mockApplyAppUpdate,
  downloadAppUpdate: hoisted.mockDownloadAppUpdate,
  installAppUpdate: hoisted.mockInstallAppUpdate,
  isTauri: hoisted.mockIsTauri,
}));

vi.mock('../../../../utils/openUrl', () => ({ openUrl: hoisted.mockOpenUrl }));

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

describe('AboutPanel', () => {
  beforeEach(() => {
    statusListeners.length = 0;
    mockCheckAppUpdate.mockReset();
    mockOpenUrl.mockReset();
    hoisted.mockApplyAppUpdate.mockReset();
    hoisted.mockDownloadAppUpdate.mockReset();
    hoisted.mockInstallAppUpdate.mockReset();
    hoisted.mockIsTauri.mockReturnValue(true);
  });

  it('renders the running app version + check button + releases link', () => {
    renderWithProviders(<AboutPanel />);

    // The test config stubs APP_VERSION to '0.0.0-test'.
    expect(screen.getByText('v0.0.0-test')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Check for updates/ })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Open GitHub releases/ })).toBeInTheDocument();
  });

  it('clicks "Check for updates" → calls checkAppUpdate and records timestamp', async () => {
    mockCheckAppUpdate.mockResolvedValueOnce({
      current_version: '0.50.0',
      available: false,
      available_version: null,
      body: null,
    });

    renderWithProviders(<AboutPanel />);

    const checkBtn = screen.getByRole('button', { name: /Check for updates/ });
    fireEvent.click(checkBtn);

    await waitFor(() => {
      expect(mockCheckAppUpdate).toHaveBeenCalledTimes(1);
    });
    // After a successful check, the panel records "Last checked …".
    await waitFor(() => {
      expect(screen.getByText(/Last checked/i)).toBeInTheDocument();
    });
  });

  it('shows the "ready to install" summary once the hook hits ready_to_install', async () => {
    renderWithProviders(<AboutPanel />);
    await waitFor(() => expect(statusListeners.length).toBeGreaterThan(0));

    emitStatus('ready_to_install');

    await waitFor(() => {
      expect(
        screen.getByText(/downloaded and ready|Use the prompt at the bottom right to restart/i)
      ).toBeInTheDocument();
    });
  });

  it('shows the up-to-date summary after a check finds no update', async () => {
    mockCheckAppUpdate.mockResolvedValueOnce({
      current_version: '0.50.0',
      available: false,
      available_version: null,
      body: null,
    });

    renderWithProviders(<AboutPanel />);

    fireEvent.click(screen.getByRole('button', { name: /Check for updates/ }));

    await waitFor(() => {
      expect(screen.getByText(/You are running the latest version/i)).toBeInTheDocument();
    });
  });

  it('clicking "Open GitHub releases" calls openUrl with the configured URL', () => {
    renderWithProviders(<AboutPanel />);

    fireEvent.click(screen.getByRole('button', { name: /Open GitHub releases/ }));

    expect(mockOpenUrl).toHaveBeenCalledTimes(1);
    expect(mockOpenUrl.mock.calls[0][0]).toEqual(
      expect.stringContaining('github.com/tinyhumansai/openhuman')
    );
  });

  it('shows the error summary when the check throws', async () => {
    mockCheckAppUpdate.mockRejectedValueOnce(new Error('endpoint unreachable'));

    renderWithProviders(<AboutPanel />);

    fireEvent.click(screen.getByRole('button', { name: /Check for updates/ }));

    await waitFor(() => {
      expect(screen.getByText(/endpoint unreachable/)).toBeInTheDocument();
    });
  });
});
