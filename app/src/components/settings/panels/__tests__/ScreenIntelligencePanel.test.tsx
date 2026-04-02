import { configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import accessibilityReducer from '../../../../store/accessibilitySlice';
import authReducer from '../../../../store/authSlice';
import socketReducer from '../../../../store/socketSlice';
import teamReducer from '../../../../store/teamSlice';
import userReducer from '../../../../store/userSlice';
import {
  type AccessibilityStatus,
  type AccessibilityVisionRecentResult,
  type CommandResponse,
  type ConfigSnapshot,
  isTauri,
  openhumanAccessibilityInputAction,
  openhumanAccessibilityRequestPermission,
  openhumanAccessibilityRequestPermissions,
  openhumanAccessibilityStartSession,
  openhumanAccessibilityStatus,
  openhumanAccessibilityStopSession,
  openhumanAccessibilityVisionFlush,
  openhumanAccessibilityVisionRecent,
  openhumanScreenIntelligenceCaptureTest,
  openhumanUpdateScreenIntelligenceSettings,
  restartCoreProcess,
} from '../../../../utils/tauriCommands';
import ScreenIntelligencePanel from '../ScreenIntelligencePanel';

vi.mock('../../../../utils/tauriCommands', () => ({
  isTauri: vi.fn(() => true),
  openhumanAccessibilityInputAction: vi.fn(),
  openhumanAccessibilityRequestPermission: vi.fn(),
  openhumanAccessibilityRequestPermissions: vi.fn(),
  openhumanAccessibilityStartSession: vi.fn(),
  openhumanAccessibilityStatus: vi.fn(),
  openhumanAccessibilityStopSession: vi.fn(),
  openhumanAccessibilityVisionFlush: vi.fn(),
  openhumanAccessibilityVisionRecent: vi.fn(),
  openhumanScreenIntelligenceCaptureTest: vi.fn(),
  openhumanUpdateScreenIntelligenceSettings: vi.fn(),
  restartCoreProcess: vi.fn(),
}));

const baseStatus: AccessibilityStatus = {
  platform_supported: true,
  permissions: {
    screen_recording: 'granted',
    accessibility: 'granted',
    input_monitoring: 'unknown',
  },
  features: { screen_monitoring: true, device_control: true, predictive_input: true },
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
    allowlist: ['Code'],
    denylist: ['1Password'],
  },
  denylist: ['1Password'],
  is_context_blocked: false,
  permission_check_process_path: '/tmp/openhuman-core',
};

const emptyVisionResponse: CommandResponse<AccessibilityVisionRecentResult> = {
  result: { summaries: [] },
  logs: [],
};

const createStore = () =>
  configureStore({
    reducer: {
      auth: authReducer,
      socket: socketReducer,
      user: userReducer,
      team: teamReducer,
      accessibility: accessibilityReducer,
    },
  });

function renderPanel() {
  const store = createStore();
  render(
    <Provider store={store}>
      <MemoryRouter initialEntries={['/settings/screen-intelligence']}>
        <ScreenIntelligencePanel />
      </MemoryRouter>
    </Provider>
  );
  return store;
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
    vi.mocked(isTauri).mockReturnValue(true);
    vi.mocked(openhumanAccessibilityStatus).mockResolvedValue({ result: baseStatus, logs: [] });
    vi.mocked(openhumanAccessibilityVisionRecent).mockResolvedValue(emptyVisionResponse);
    vi.mocked(openhumanAccessibilityInputAction).mockResolvedValue({
      result: {} as never,
      logs: [],
    });
    vi.mocked(openhumanAccessibilityRequestPermission).mockResolvedValue({
      result: baseStatus.permissions,
      logs: [],
    } as never);
    vi.mocked(openhumanAccessibilityRequestPermissions).mockResolvedValue({
      result: baseStatus.permissions,
      logs: [],
    } as never);
    vi.mocked(openhumanAccessibilityStartSession).mockResolvedValue({
      result: baseStatus.session,
      logs: [],
    } as never);
    vi.mocked(openhumanAccessibilityStopSession).mockResolvedValue({
      result: baseStatus.session,
      logs: [],
    } as never);
    vi.mocked(openhumanAccessibilityVisionFlush).mockResolvedValue({
      result: { accepted: true, summary: null },
      logs: [],
    } as never);
    vi.mocked(openhumanScreenIntelligenceCaptureTest).mockResolvedValue({
      result: {
        ok: false,
        capture_mode: 'fullscreen',
        context: null,
        image_ref: null,
        bytes_estimate: null,
        error: 'screen capture is unsupported on this platform',
        timing_ms: 12,
      },
      logs: [],
    });
    vi.mocked(restartCoreProcess).mockResolvedValue(undefined);
  });

  it('saves screen intelligence settings and clears the saving state', async () => {
    const deferred = createDeferred<CommandResponse<ConfigSnapshot>>();
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
    expect(openhumanAccessibilityStatus).toHaveBeenCalledTimes(2);
  });

  it('shows permission restart guidance and unsupported-platform messaging', async () => {
    vi.mocked(openhumanAccessibilityStatus).mockResolvedValueOnce({
      result: {
        ...baseStatus,
        platform_supported: false,
        permissions: {
          screen_recording: 'denied',
          accessibility: 'denied',
          input_monitoring: 'unknown',
        },
      },
      logs: [],
    });

    renderPanel();

    expect(await screen.findByText('Permissions')).toBeInTheDocument();
    expect(screen.getByText(/After granting in System Settings, click/i)).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Restart & Refresh Permissions' })
    ).toBeInTheDocument();
    expect(
      screen.getByText('Screen Intelligence V1 is currently supported on macOS only.')
    ).toBeInTheDocument();
  });
});
