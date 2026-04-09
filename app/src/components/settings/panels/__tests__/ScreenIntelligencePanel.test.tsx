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

  it('saves screen intelligence settings and refreshes core-backed status', async () => {
    const deferred = createDeferred<{ result: ConfigSnapshot; logs: [] }>();
    vi.mocked(openhumanUpdateScreenIntelligenceSettings).mockReturnValueOnce(deferred.promise);

    renderPanel();

    await screen.findByText('Screen Intelligence Policy');

    const enabledLabel = screen.getByText('Enabled').closest('label');
    const enabledCheckbox = enabledLabel?.querySelector(
      'input[type="checkbox"]'
    ) as HTMLInputElement;
    expect(enabledCheckbox.checked).toBe(false);

    fireEvent.click(enabledCheckbox);
    fireEvent.click(screen.getByRole('button', { name: 'Save Screen Intelligence Settings' }));

    expect(await screen.findByRole('button', { name: 'Saving…' })).toBeInTheDocument();
    expect(openhumanUpdateScreenIntelligenceSettings).toHaveBeenCalledWith({
      enabled: true,
      policy_mode: 'all_except_blacklist',
      baseline_fps: 1,
      use_vision_model: true,
      keep_screenshots: false,
      allowlist: ['Code'],
      denylist: ['1Password'],
    });

    deferred.resolve({
      result: { config: {}, workspace_dir: '/tmp/workspace', config_path: '/tmp/config.toml' },
      logs: [],
    });

    await waitFor(() => {
      expect(
        screen.getByRole('button', { name: 'Save Screen Intelligence Settings' })
      ).toBeInTheDocument();
    });
    expect(baseState.refreshStatus).toHaveBeenCalledTimes(1);
  });

  it('shows permission restart guidance and unsupported-platform messaging', async () => {
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

    expect(await screen.findByText('Permissions')).toBeInTheDocument();
    expect(screen.getByText(/After granting in System Settings, click/i)).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Restart & Refresh Permissions' })
    ).toBeInTheDocument();
    expect(
      screen.getByText('Screen Intelligence V1 is currently supported on macOS only.')
    ).toBeInTheDocument();
  });

  it('shows the last successful restart summary', async () => {
    renderPanel({
      ...baseState,
      lastRestartSummary: 'Core restarted: PID 4000 at 9:00:00 AM -> PID 4242 at 9:01:00 AM.',
    });

    expect(await screen.findByText(/Core restarted: PID 4000/i)).toBeInTheDocument();
  });
});
