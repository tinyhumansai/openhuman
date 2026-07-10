import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  type ScreenIntelligenceState,
  useScreenIntelligenceState,
} from '../../../../features/screen-intelligence/useScreenIntelligenceState';
import {
  type ConfigSnapshot,
  isTauri,
  openhumanUpdateScreenIntelligenceSettings,
} from '../../../../utils/tauriCommands';
import ScreenIntelligencePanel from '../ScreenIntelligencePanel';

vi.mock('../../../../features/screen-intelligence/useScreenIntelligenceState', () => ({
  useScreenIntelligenceState: vi.fn(),
}));

vi.mock('../../../../utils/tauriCommands', async importOriginal => {
  const actual = await importOriginal<typeof import('../../../../utils/tauriCommands')>();
  return {
    ...actual,
    isTauri: vi.fn(() => true),
    openhumanUpdateScreenIntelligenceSettings: vi.fn(),
  };
});

const baseState: ScreenIntelligenceState = {
  status: {
    platform_supported: true,
    core_process: { pid: 4242, started_at_ms: 1712700000000 },
    permissions: {
      screen_recording: 'granted',
      accessibility: 'granted',
      input_monitoring: 'unknown',
    },
    features: { screen_monitoring: true },
    session: {
      active: false,
      started_at_ms: null,
      expires_at_ms: null,
      remaining_ms: null,
      ttl_secs: 300,
      panic_hotkey: 'Cmd+Shift+.',
      stop_reason: null,
      frames_in_memory: 0,
      last_capture_at_ms: null,
      last_context: null,
      vision_enabled: true,
      vision_state: 'idle',
      vision_queue_depth: 0,
      last_vision_at_ms: null,
      last_vision_summary: null,
    },
    config: {
      enabled: false,
      capture_policy: 'hybrid',
      policy_mode: 'all_except_blacklist',
      baseline_fps: 1,
      vision_enabled: true,
      session_ttl_secs: 300,
      panic_stop_hotkey: 'Cmd+Shift+.',
      autocomplete_enabled: true,
      use_vision_model: true,
      keep_screenshots: false,
      allowlist: ['Code'],
      denylist: ['1Password'],
    },
    denylist: ['1Password'],
    is_context_blocked: false,
    permission_check_process_path: '/tmp/openhuman-core',
  },
  lastRestartSummary: null,
  recentVisionSummaries: [],
  captureTestResult: null,
  isCaptureTestRunning: false,
  isLoading: false,
  isRequestingPermissions: false,
  isRestartingCore: false,
  isStartingSession: false,
  isStoppingSession: false,
  isLoadingVision: false,
  isFlushingVision: false,
  lastError: null,
  refreshStatus: vi.fn().mockResolvedValue(null),
  requestPermission: vi.fn().mockResolvedValue(null),
  refreshPermissionsWithRestart: vi.fn().mockResolvedValue(null),
  startSession: vi.fn().mockResolvedValue(null),
  stopSession: vi.fn().mockResolvedValue(null),
  refreshVision: vi.fn().mockResolvedValue([]),
  flushVision: vi.fn().mockResolvedValue(undefined),
  runCaptureTest: vi.fn().mockResolvedValue(undefined),
  clearError: vi.fn(),
};

function renderPanel(state: ScreenIntelligenceState = baseState) {
  vi.mocked(useScreenIntelligenceState).mockReturnValue(state);
  render(
    <MemoryRouter initialEntries={['/settings/screen-intelligence']}>
      <ScreenIntelligencePanel />
    </MemoryRouter>
  );
}

function createDeferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(res => {
    resolve = res;
  });
  return { promise, resolve };
}

describe('ScreenIntelligencePanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(isTauri).mockReturnValue(true);
  });

  it('saves screen awareness settings and refreshes core-backed status', async () => {
    const deferred = createDeferred<{ result: ConfigSnapshot; logs: [] }>();
    vi.mocked(openhumanUpdateScreenIntelligenceSettings).mockReturnValueOnce(deferred.promise);

    renderPanel();

    // The SettingsHeader renders "Screen Awareness" as the panel heading; wait for it.
    await waitFor(() => {
      expect(screen.getAllByText('Screen Awareness').length).toBeGreaterThan(0);
    });

    const enabledSwitch = screen.getByRole('switch', { name: 'Enabled' });
    expect(enabledSwitch).toHaveAttribute('aria-checked', 'false');

    fireEvent.click(enabledSwitch);
    fireEvent.click(screen.getByRole('button', { name: 'Save Settings' }));

    expect(await screen.findByRole('button', { name: 'Saving…' })).toBeInTheDocument();
    expect(openhumanUpdateScreenIntelligenceSettings).toHaveBeenCalledWith(
      expect.objectContaining({ enabled: true, policy_mode: 'all_except_blacklist' })
    );

    deferred.resolve({
      result: { config: {}, workspace_dir: '/tmp/workspace', config_path: '/tmp/config.toml' },
      logs: [],
    });

    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Save Settings' })).toBeInTheDocument();
    });
    expect(baseState.refreshStatus).toHaveBeenCalledTimes(1);
  });

  it('hides permissions section and shows unsupported-platform messaging', async () => {
    renderPanel({
      ...baseState,
      status: {
        ...baseState.status!,
        platform_supported: false,
        permissions: {
          screen_recording: 'denied',
          accessibility: 'denied',
          input_monitoring: 'unknown',
        },
      },
    });

    await waitFor(() => {
      expect(
        screen.getByText(
          'Screen Awareness desktop capture and permission controls are currently supported on macOS only.'
        )
      ).toBeInTheDocument();
    });
    expect(screen.queryByText('Permissions')).not.toBeInTheDocument();
    expect(screen.queryByText(/After granting in System Settings, click/i)).not.toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: 'Restart & Refresh Permissions' })
    ).not.toBeInTheDocument();
  });

  it('shows the last successful restart summary', async () => {
    renderPanel({
      ...baseState,
      lastRestartSummary: 'Core restarted: PID 4000 at 9:00:00 AM -> PID 4242 at 9:01:00 AM.',
    });

    expect(await screen.findByText(/Core restarted: PID 4000/i)).toBeInTheDocument();
  });

  // ─── Session controls ─────────────────────────────────────────────────────

  it('Start Session button calls startSession with consent=true', async () => {
    const startSession = vi.fn().mockResolvedValue(null);
    renderPanel({
      ...baseState,
      status: {
        ...baseState.status!,
        // accessibility granted so Start is not disabled
        permissions: { ...baseState.status!.permissions, accessibility: 'granted' },
        session: { ...baseState.status!.session, active: false },
      },
      startSession,
    });

    await waitFor(() => expect(screen.getAllByText(/start session/i).length).toBeGreaterThan(0));

    fireEvent.click(screen.getByRole('button', { name: /start session/i }));

    await waitFor(() =>
      expect(startSession).toHaveBeenCalledWith(expect.objectContaining({ consent: true }))
    );
  });

  it('Stop Session button calls stopSession', async () => {
    const stopSession = vi.fn().mockResolvedValue(null);
    renderPanel({
      ...baseState,
      status: { ...baseState.status!, session: { ...baseState.status!.session, active: true } },
      stopSession,
    });

    await waitFor(() =>
      expect(screen.getByRole('button', { name: /stop session/i })).not.toBeDisabled()
    );

    fireEvent.click(screen.getByRole('button', { name: /stop session/i }));

    await waitFor(() => expect(stopSession).toHaveBeenCalledWith('manual_stop'));
  });

  it('Analyze Now button calls flushVision when session is active', async () => {
    const flushVision = vi.fn().mockResolvedValue(undefined);
    renderPanel({
      ...baseState,
      status: { ...baseState.status!, session: { ...baseState.status!.session, active: true } },
      flushVision,
    });

    await waitFor(() =>
      expect(screen.getByRole('button', { name: /analyze now/i })).not.toBeDisabled()
    );

    fireEvent.click(screen.getByRole('button', { name: /analyze now/i }));

    await waitFor(() => expect(flushVision).toHaveBeenCalled());
  });

  it('Analyze Now is disabled when session is not active', async () => {
    renderPanel({
      ...baseState,
      status: { ...baseState.status!, session: { ...baseState.status!.session, active: false } },
    });

    await waitFor(() =>
      expect(screen.getByRole('button', { name: /analyze now/i })).toBeDisabled()
    );
  });

  // ─── Policy mode select ───────────────────────────────────────────────────

  it('changing policy mode select updates the local state', async () => {
    renderPanel();

    await waitFor(() => expect(screen.getAllByText(/screen awareness/i).length).toBeGreaterThan(0));

    const policySelect = screen.getByRole('combobox', { name: /mode/i }) as HTMLSelectElement;

    fireEvent.change(policySelect, { target: { value: 'whitelist_only' } });

    expect(policySelect.value).toBe('whitelist_only');
  });

  it('saves with whitelist_only policy when mode is changed', async () => {
    const deferred = createDeferred<{ result: ConfigSnapshot; logs: [] }>();
    vi.mocked(openhumanUpdateScreenIntelligenceSettings).mockReturnValueOnce(deferred.promise);

    renderPanel();

    await waitFor(() => expect(screen.getAllByText(/screen awareness/i).length).toBeGreaterThan(0));

    const policySelect = screen.getByRole('combobox', { name: /mode/i });
    fireEvent.change(policySelect, { target: { value: 'whitelist_only' } });

    fireEvent.click(screen.getByRole('button', { name: 'Save Settings' }));

    expect(vi.mocked(openhumanUpdateScreenIntelligenceSettings)).toHaveBeenCalledWith(
      expect.objectContaining({ policy_mode: 'whitelist_only' })
    );

    deferred.resolve({
      result: { config: {}, workspace_dir: '/tmp/workspace', config_path: '/tmp/config.toml' },
      logs: [],
    });
  });

  // ─── Error display ────────────────────────────────────────────────────────

  it('shows lastError when state has an error', async () => {
    renderPanel({ ...baseState, lastError: 'Permission denied by OS' });

    expect(await screen.findByText('Permission denied by OS')).toBeInTheDocument();
  });

  // ─── Screen monitoring toggle ─────────────────────────────────────────────

  it('toggling screen monitoring checkbox updates the override', async () => {
    renderPanel();

    await waitFor(() => expect(screen.getAllByText(/screen awareness/i).length).toBeGreaterThan(0));

    const monitoringSwitch = screen.getByRole('switch', { name: 'Screen Monitoring' });

    // Initially reflects status value (true in baseState.features)
    expect(monitoringSwitch).toHaveAttribute('aria-checked', 'true');

    fireEvent.click(monitoringSwitch);
    expect(monitoringSwitch).toHaveAttribute('aria-checked', 'false');
  });

  // ─── Session status display ───────────────────────────────────────────────

  it('shows Active session status when session is active', async () => {
    renderPanel({
      ...baseState,
      status: { ...baseState.status!, session: { ...baseState.status!.session, active: true } },
    });

    await waitFor(() => expect(screen.getByText(/active/i)).toBeInTheDocument());
  });

  it('shows Stopped session status when session is inactive', async () => {
    renderPanel({
      ...baseState,
      status: { ...baseState.status!, session: { ...baseState.status!.session, active: false } },
    });

    await waitFor(() => expect(screen.getByText(/stopped/i)).toBeInTheDocument());
  });
});
