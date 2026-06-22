import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  fetchScreenIntelligenceStatus,
  refreshScreenIntelligencePermissionsWithRestart,
  requestScreenIntelligencePermission,
} from '../../../../features/screen-intelligence/api';
import {
  type AccessibilityPermissionState,
  type AccessibilityStatus,
  openhumanGetAutonomySettings,
  openhumanGetVoiceServerSettings,
  openhumanUpdateAutonomySettings,
  openhumanUpdateVoiceServerSettings,
  syncNotchVisibility,
} from '../../../../utils/tauriCommands';
import DesktopAgentPanel from '../DesktopAgentPanel';

vi.mock('../../../../features/screen-intelligence/api', () => ({
  fetchScreenIntelligenceStatus: vi.fn(),
  requestScreenIntelligencePermission: vi.fn(),
  refreshScreenIntelligencePermissionsWithRestart: vi.fn(),
}));

vi.mock('../../../../utils/tauriCommands', async importOriginal => {
  const actual = await importOriginal<typeof import('../../../../utils/tauriCommands')>();
  return {
    ...actual,
    openhumanGetAutonomySettings: vi.fn(),
    openhumanUpdateAutonomySettings: vi.fn(),
    openhumanGetVoiceServerSettings: vi.fn(),
    openhumanUpdateVoiceServerSettings: vi.fn(),
    syncNotchVisibility: vi.fn(),
  };
});

function mockVoice(alwaysOn: boolean) {
  vi.mocked(openhumanGetVoiceServerSettings).mockResolvedValue({
    result: { always_on_enabled: alwaysOn },
    logs: [],
  } as unknown as Awaited<ReturnType<typeof openhumanGetVoiceServerSettings>>);
}

const SEAMLESS_TOOLS = ['automate', 'ax_interact', 'launch_app', 'keyboard', 'mouse'];

function mockAutonomy(autoApprove: string[]) {
  vi.mocked(openhumanGetAutonomySettings).mockResolvedValue({
    result: { auto_approve: autoApprove },
    logs: [],
  } as unknown as Awaited<ReturnType<typeof openhumanGetAutonomySettings>>);
}

type Perms = {
  microphone: AccessibilityPermissionState;
  accessibility: AccessibilityPermissionState;
  screen_recording: AccessibilityPermissionState;
  input_monitoring: AccessibilityPermissionState;
};

function makeStatus(permissions: Perms): AccessibilityStatus {
  return {
    platform_supported: true,
    permissions,
    permission_check_process_path: '/tmp/openhuman-core',
  } as unknown as AccessibilityStatus;
}

function renderPanel() {
  render(
    <MemoryRouter initialEntries={['/settings/desktop-agent']}>
      <DesktopAgentPanel />
    </MemoryRouter>
  );
}

describe('DesktopAgentPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockAutonomy([]);
    mockVoice(false);
    vi.mocked(openhumanUpdateAutonomySettings).mockResolvedValue(
      {} as unknown as Awaited<ReturnType<typeof openhumanUpdateAutonomySettings>>
    );
    vi.mocked(openhumanUpdateVoiceServerSettings).mockResolvedValue(
      {} as unknown as Awaited<ReturnType<typeof openhumanUpdateVoiceServerSettings>>
    );
    vi.mocked(syncNotchVisibility).mockResolvedValue(undefined);
  });

  it('renders a row per permission with the right grant affordance', async () => {
    vi.mocked(fetchScreenIntelligenceStatus).mockResolvedValue(
      makeStatus({
        microphone: 'granted',
        accessibility: 'denied',
        screen_recording: 'unknown',
        input_monitoring: 'unsupported',
      })
    );

    renderPanel();

    // Granted permission → badge, no grant button.
    const micRow = await screen.findByTestId('desktop-agent-perm-microphone');
    expect(within(micRow).getByText('granted')).toBeInTheDocument();
    expect(screen.queryByTestId('desktop-agent-grant-microphone')).not.toBeInTheDocument();

    // Denied + unknown → grant buttons.
    expect(screen.getByTestId('desktop-agent-grant-accessibility')).toBeInTheDocument();
    expect(screen.getByTestId('desktop-agent-grant-screen_recording')).toBeInTheDocument();

    // Unsupported → no grant button (muted "not required" instead).
    expect(screen.queryByTestId('desktop-agent-grant-input_monitoring')).not.toBeInTheDocument();
    const inputRow = screen.getByTestId('desktop-agent-perm-input_monitoring');
    expect(within(inputRow).getByText('Not required on this OS')).toBeInTheDocument();
  });

  it('requests the permission and reflects the new status when Grant is clicked', async () => {
    vi.mocked(fetchScreenIntelligenceStatus).mockResolvedValue(
      makeStatus({
        microphone: 'granted',
        accessibility: 'denied',
        screen_recording: 'granted',
        input_monitoring: 'unsupported',
      })
    );
    vi.mocked(requestScreenIntelligencePermission).mockResolvedValue(
      makeStatus({
        microphone: 'granted',
        accessibility: 'granted',
        screen_recording: 'granted',
        input_monitoring: 'unsupported',
      })
    );

    renderPanel();

    const grantBtn = await screen.findByTestId('desktop-agent-grant-accessibility');
    fireEvent.click(grantBtn);

    await waitFor(() =>
      expect(requestScreenIntelligencePermission).toHaveBeenCalledWith('accessibility')
    );
    // Once granted, the grant button disappears and the all-granted banner shows.
    await waitFor(() =>
      expect(screen.queryByTestId('desktop-agent-grant-accessibility')).not.toBeInTheDocument()
    );
    expect(screen.getByTestId('desktop-agent-all-granted')).toBeInTheDocument();
  });

  it('surfaces an error when a permission request fails', async () => {
    vi.mocked(fetchScreenIntelligenceStatus).mockResolvedValue(
      makeStatus({
        microphone: 'denied',
        accessibility: 'granted',
        screen_recording: 'granted',
        input_monitoring: 'unsupported',
      })
    );
    vi.mocked(requestScreenIntelligencePermission).mockRejectedValue(
      new Error('permission request blew up')
    );

    renderPanel();

    fireEvent.click(await screen.findByTestId('desktop-agent-grant-microphone'));

    const errorBox = await screen.findByTestId('desktop-agent-error');
    expect(errorBox).toHaveTextContent('permission request blew up');
    // refresh-with-restart is unrelated to a failed grant.
    expect(refreshScreenIntelligencePermissionsWithRestart).not.toHaveBeenCalled();
  });

  describe('seamless mode (act without asking)', () => {
    beforeEach(() => {
      vi.mocked(fetchScreenIntelligenceStatus).mockResolvedValue(
        makeStatus({
          microphone: 'granted',
          accessibility: 'granted',
          screen_recording: 'granted',
          input_monitoring: 'unsupported',
        })
      );
    });

    it('reflects the auto-approve allowlist: off when the desktop tools are absent', async () => {
      mockAutonomy([]);
      renderPanel();
      const toggle = await screen.findByTestId('desktop-agent-seamless-toggle');
      await waitFor(() => expect(toggle).toHaveAttribute('aria-checked', 'false'));
    });

    it('reflects the auto-approve allowlist: on when all desktop tools are present', async () => {
      mockAutonomy([...SEAMLESS_TOOLS]);
      renderPanel();
      const toggle = await screen.findByTestId('desktop-agent-seamless-toggle');
      await waitFor(() => expect(toggle).toHaveAttribute('aria-checked', 'true'));
    });

    it('enabling grants Full access, drops plan approval, and auto-approves the desktop tools', async () => {
      mockAutonomy([]);
      renderPanel();
      const toggle = await screen.findByTestId('desktop-agent-seamless-toggle');
      await waitFor(() => expect(toggle).toHaveAttribute('aria-checked', 'false'));

      fireEvent.click(toggle);

      await waitFor(() => expect(openhumanUpdateAutonomySettings).toHaveBeenCalledTimes(1));
      const payload = vi.mocked(openhumanUpdateAutonomySettings).mock.calls[0][0];
      expect(payload).toMatchObject({ level: 'full', require_task_plan_approval: false });
      expect(payload.auto_approve).toEqual(expect.arrayContaining(SEAMLESS_TOOLS));
    });

    it('disabling removes the desktop tools and restores plan approval, keeping other allows', async () => {
      mockAutonomy([...SEAMLESS_TOOLS, 'shell']);
      renderPanel();
      const toggle = await screen.findByTestId('desktop-agent-seamless-toggle');
      await waitFor(() => expect(toggle).toHaveAttribute('aria-checked', 'true'));

      fireEvent.click(toggle);

      await waitFor(() => expect(openhumanUpdateAutonomySettings).toHaveBeenCalledTimes(1));
      const payload = vi.mocked(openhumanUpdateAutonomySettings).mock.calls[0][0];
      expect(payload).toMatchObject({ require_task_plan_approval: true });
      expect(payload.auto_approve).toEqual(['shell']);
      // Disabling must not downgrade the tier.
      expect(payload.level).toBeUndefined();
    });
  });

  describe('always-on listening (relocated from Voice)', () => {
    beforeEach(() => {
      vi.mocked(fetchScreenIntelligenceStatus).mockResolvedValue(
        makeStatus({
          microphone: 'granted',
          accessibility: 'granted',
          screen_recording: 'granted',
          input_monitoring: 'unsupported',
        })
      );
    });

    it('reflects the persisted always-on flag', async () => {
      mockVoice(true);
      renderPanel();
      const toggle = await screen.findByTestId('voice-always-on-toggle');
      await waitFor(() => expect(toggle).toHaveAttribute('aria-checked', 'true'));
    });

    it('persists the flag and syncs the notch when toggled on', async () => {
      mockVoice(false);
      renderPanel();
      const toggle = await screen.findByTestId('voice-always-on-toggle');
      await waitFor(() => expect(toggle).toHaveAttribute('aria-checked', 'false'));

      fireEvent.click(toggle);

      await waitFor(() =>
        expect(openhumanUpdateVoiceServerSettings).toHaveBeenCalledWith({ always_on_enabled: true })
      );
      await waitFor(() => expect(syncNotchVisibility).toHaveBeenCalledWith(true));
    });

    it('reverts and never touches the notch when the update fails', async () => {
      mockVoice(false);
      vi.mocked(openhumanUpdateVoiceServerSettings).mockRejectedValueOnce(new Error('rpc down'));
      renderPanel();
      const toggle = await screen.findByTestId('voice-always-on-toggle');
      await waitFor(() => expect(toggle).toHaveAttribute('aria-checked', 'false'));

      fireEvent.click(toggle);

      await waitFor(() => expect(toggle).toHaveAttribute('aria-checked', 'false'));
      expect(syncNotchVisibility).not.toHaveBeenCalled();
    });
  });
});
