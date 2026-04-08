import { configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import accessibilityReducer from '../../../store/accessibilitySlice';
import authReducer from '../../../store/authSlice';
import socketReducer from '../../../store/socketSlice';
import teamReducer from '../../../store/teamSlice';
import userReducer from '../../../store/userSlice';
import {
  type AccessibilityStatus,
  type AccessibilityVisionRecentResult,
  type CaptureTestResult,
  type CommandResponse,
  openhumanAccessibilityInputAction,
  openhumanAccessibilityRequestPermission,
  openhumanAccessibilityRequestPermissions,
  openhumanAccessibilityStartSession,
  openhumanAccessibilityStatus,
  openhumanAccessibilityStopSession,
  openhumanAccessibilityVisionFlush,
  openhumanAccessibilityVisionRecent,
  openhumanScreenIntelligenceCaptureTest,
  restartCoreProcess,
} from '../../../utils/tauriCommands';
import ScreenIntelligenceDebugPanel from '../ScreenIntelligenceDebugPanel';

vi.mock('../../../utils/tauriCommands', () => ({
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
  restartCoreProcess: vi.fn(),
}));

const sampleStatus: AccessibilityStatus = {
  platform_supported: true,
  permissions: {
    screen_recording: 'granted',
    accessibility: 'granted',
    input_monitoring: 'granted',
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
    enabled: true,
    capture_policy: 'hybrid',
    policy_mode: 'all_except_blacklist',
    baseline_fps: 1,
    vision_enabled: true,
    session_ttl_secs: 300,
    panic_stop_hotkey: 'Cmd+Shift+.',
    autocomplete_enabled: true,
    use_vision_model: true,
    keep_screenshots: false,
    allowlist: [],
    denylist: [],
  },
  denylist: [],
  is_context_blocked: false,
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
      <ScreenIntelligenceDebugPanel />
    </Provider>
  );
  return store;
}

describe('ScreenIntelligenceDebugPanel', () => {
  beforeEach(() => {
    vi.mocked(openhumanAccessibilityStatus).mockResolvedValue({ result: sampleStatus, logs: [] });
    vi.mocked(openhumanAccessibilityVisionRecent).mockResolvedValue(emptyVisionResponse);
    vi.mocked(openhumanAccessibilityInputAction).mockResolvedValue({
      result: {} as never,
      logs: [],
    });
    vi.mocked(openhumanAccessibilityRequestPermission).mockResolvedValue({
      result: sampleStatus.permissions,
      logs: [],
    } as never);
    vi.mocked(openhumanAccessibilityRequestPermissions).mockResolvedValue({
      result: sampleStatus.permissions,
      logs: [],
    } as never);
    vi.mocked(openhumanAccessibilityStartSession).mockResolvedValue({
      result: sampleStatus.session,
      logs: [],
    } as never);
    vi.mocked(openhumanAccessibilityStopSession).mockResolvedValue({
      result: sampleStatus.session,
      logs: [],
    } as never);
    vi.mocked(openhumanAccessibilityVisionFlush).mockResolvedValue({
      result: { accepted: true, summary: null },
      logs: [],
    } as never);
    vi.mocked(restartCoreProcess).mockResolvedValue(undefined);
  });

  it('renders successful capture diagnostics and preview image', async () => {
    const captureResult: CaptureTestResult = {
      ok: true,
      capture_mode: 'windowed',
      context: {
        app_name: 'Safari',
        window_title: 'GitHub',
        bounds_x: 10,
        bounds_y: 20,
        bounds_width: 1440,
        bounds_height: 900,
      },
      image_ref: 'data:image/png;base64,ZmFrZQ==',
      bytes_estimate: 2048,
      error: null,
      timing_ms: 155,
    };
    vi.mocked(openhumanScreenIntelligenceCaptureTest).mockResolvedValue({
      result: captureResult,
      logs: [],
    });

    renderPanel();

    await waitFor(() => {
      expect(openhumanAccessibilityStatus).toHaveBeenCalled();
      expect(openhumanAccessibilityVisionRecent).toHaveBeenCalledWith(5);
    });

    fireEvent.click(screen.getByRole('button', { name: 'Test Capture' }));

    expect(await screen.findByText('Success')).toBeInTheDocument();
    expect(screen.getByText('windowed')).toBeInTheDocument();
    expect(screen.getByText('155ms')).toBeInTheDocument();
    expect(screen.getByText('2.0 KB')).toBeInTheDocument();
    expect(screen.getByText('Safari')).toBeInTheDocument();
    expect(screen.getByAltText('Capture test result')).toHaveAttribute(
      'src',
      'data:image/png;base64,ZmFrZQ=='
    );
  });

  it('renders capture failures without breaking the diagnostics panel', async () => {
    vi.mocked(openhumanScreenIntelligenceCaptureTest).mockResolvedValue({
      result: {
        ok: false,
        capture_mode: 'fullscreen',
        context: null,
        image_ref: null,
        bytes_estimate: null,
        error: 'screen recording permission is not granted',
        timing_ms: 42,
      },
      logs: [],
    });

    renderPanel();

    fireEvent.click(screen.getByRole('button', { name: 'Test Capture' }));

    expect(await screen.findByText('Failed')).toBeInTheDocument();
    expect(screen.getByText('fullscreen')).toBeInTheDocument();
    expect(screen.getByText('42ms')).toBeInTheDocument();
    expect(screen.getByText('screen recording permission is not granted')).toBeInTheDocument();
    expect(screen.queryByAltText('Capture test result')).not.toBeInTheDocument();
    expect(screen.getByText('Permissions')).toBeInTheDocument();
  });
});
