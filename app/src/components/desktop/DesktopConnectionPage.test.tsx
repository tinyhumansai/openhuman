// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc, isLocalDesktopHost } from '../../services/coreRpcClient';
import { renderWithProviders } from '../../test/test-utils';
import { openUrl } from '../../utils/openUrl';
import DesktopConnectionPage, { type DesktopStatus } from './DesktopConnectionPage';

vi.mock('../../services/coreRpcClient', () => ({
  callCoreRpc: vi.fn(),
  isLocalDesktopHost: vi.fn(async () => true),
}));
vi.mock('../../utils/openUrl', () => ({ openUrl: vi.fn().mockResolvedValue(undefined) }));

const enabled: DesktopStatus = {
  supported: true,
  enabled: true,
  platform: 'macos',
  module_state: 'ready',
  accessibility: 'granted',
  screen_recording: 'denied',
  jev_ready: true,
  approvals_enabled: false,
};

describe('Desktop connection', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(isLocalDesktopHost).mockResolvedValue(true);
  });
  afterEach(() => cleanup());

  it('shows the core permission state and performs a read-only probe', async () => {
    vi.mocked(callCoreRpc).mockImplementation(async ({ method }) => {
      if (method === 'openhuman.desktop_status') return enabled as never;
      if (method === 'openhuman.desktop_pending') return [] as never;
      if (method === 'openhuman.desktop_probe') return { ok: true, app_count: 2 } as never;
      throw new Error(`Unexpected method: ${method}`);
    });
    renderWithProviders(<DesktopConnectionPage />);
    expect(screen.getByText(/Early Alpha/)).toBeInTheDocument();

    expect(await screen.findByText('Screen Recording')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Open settings' }));
    expect(openUrl).toHaveBeenCalledWith(
      'x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture'
    );
    fireEvent.click(screen.getByRole('button', { name: 'Test desktop access' }));
    expect(await screen.findByText('Desktop access works.')).toBeInTheDocument();
    expect(callCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.desktop_probe' });
    expect(callCoreRpc).not.toHaveBeenCalledWith({ method: 'openhuman.desktop_pending' });
  });

  it('keeps the switch off on a failed write and surfaces the failure', async () => {
    vi.mocked(callCoreRpc).mockImplementation(async ({ method }) => {
      if (method === 'openhuman.desktop_status') return { ...enabled, enabled: false } as never;
      throw new Error('could not save setting');
    });
    renderWithProviders(<DesktopConnectionPage />);

    fireEvent.click(await screen.findByRole('switch', { name: 'Enable' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('could not save setting');
    // The switch stays off (unchecked) and still offers "Enable" since the
    // write failed and the core-reported status never flipped to enabled.
    expect(screen.getByRole('switch', { name: 'Enable' })).toBeInTheDocument();
    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.desktop_set_enabled',
      params: { enabled: true },
    });
  });

  it('does not offer enablement on unsupported runtimes', async () => {
    vi.mocked(callCoreRpc).mockResolvedValue({ ...enabled, supported: false, enabled: false });
    renderWithProviders(<DesktopConnectionPage />);
    await waitFor(() =>
      expect(
        screen.getByText('Desktop control is unavailable in this runtime.')
      ).toBeInTheDocument()
    );
    expect(screen.queryByRole('button', { name: 'Enable' })).not.toBeInTheDocument();
  });

  it('says desktop control is not available on Windows yet', async () => {
    vi.mocked(callCoreRpc).mockResolvedValue({
      ...enabled,
      supported: false,
      enabled: false,
      platform: 'windows',
    });
    renderWithProviders(<DesktopConnectionPage />);
    expect(
      await screen.findByText(
        "Desktop control isn't available on Windows yet. Browser Control works on Windows today."
      )
    ).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Enable' })).not.toBeInTheDocument();
  });

  it('does not offer a remote or browser client control over its core host', async () => {
    vi.mocked(isLocalDesktopHost).mockResolvedValue(false);
    renderWithProviders(<DesktopConnectionPage />);
    expect(
      await screen.findByText('Desktop control is unavailable in this runtime.')
    ).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Enable' })).not.toBeInTheDocument();
    expect(callCoreRpc).not.toHaveBeenCalled();
  });

  it('sends a pending action approval through the trusted core RPC', async () => {
    let approved = false;
    vi.mocked(callCoreRpc).mockImplementation(async ({ method, params }) => {
      if (method === 'openhuman.desktop_status')
        return { ...enabled, approvals_enabled: true } as never;
      if (method === 'openhuman.desktop_pending') {
        return (
          approved
            ? []
            : [
                {
                  confirmation_id: 'pending-1',
                  app: 'TextEdit',
                  goal: 'Delete a scratch note',
                  operation: 'Click',
                  target_name: 'Delete',
                  action_summary: "Click button 'Delete' in TextEdit",
                  reason: 'This action removes the note.',
                  expires_at: '2030-01-01T00:00:00Z',
                  approved: false,
                },
              ]
        ) as never;
      }
      if (method === 'openhuman.desktop_confirm') {
        expect(params).toEqual({ confirmation_id: 'pending-1', approve: true });
        approved = true;
        return { confirmation_id: 'pending-1', approve: true } as never;
      }
      throw new Error(`Unexpected method: ${method}`);
    });
    renderWithProviders(<DesktopConnectionPage />);

    expect(await screen.findByText('This action removes the note.')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Approve' }));
    await waitFor(() => expect(screen.queryByText('This action removes the note.')).toBeNull());
    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.desktop_confirm',
      params: { confirmation_id: 'pending-1', approve: true },
    });
  });

  it('surfaces a pending approval read failure instead of silently hiding it', async () => {
    vi.mocked(callCoreRpc).mockImplementation(async ({ method }) => {
      if (method === 'openhuman.desktop_status')
        return { ...enabled, approvals_enabled: true } as never;
      if (method === 'openhuman.desktop_pending') throw new Error('unavailable');
      throw new Error(`Unexpected method: ${method}`);
    });
    renderWithProviders(<DesktopConnectionPage />);
    expect(
      await screen.findByText('Could not load pending desktop approvals. Refresh to try again.')
    ).toBeInTheDocument();
  });

  it('treats not_required accessibility as ready on Windows and suppresses the settings button', async () => {
    const windowsReady: DesktopStatus = {
      ...enabled,
      platform: 'windows',
      accessibility: 'not_required',
      screen_recording: 'not_required',
    };
    vi.mocked(callCoreRpc).mockImplementation(async ({ method }) => {
      if (method === 'openhuman.desktop_status') return windowsReady as never;
      if (method === 'openhuman.desktop_pending') return [] as never;
      if (method === 'openhuman.desktop_probe') return { ok: true, app_count: 1 } as never;
      throw new Error(`Unexpected method: ${method}`);
    });
    renderWithProviders(<DesktopConnectionPage />);

    // "Not required" is a success state — the status badge should report connected.
    expect(await screen.findByText('Connected')).toBeInTheDocument();
    // No "Open settings" button should appear for permissions that are not_required.
    expect(screen.queryByRole('button', { name: 'Open settings' })).not.toBeInTheDocument();
    // The probe should succeed.
    fireEvent.click(screen.getByRole('button', { name: 'Test desktop access' }));
    expect(await screen.findByText('Desktop access works.')).toBeInTheDocument();
  });
});
