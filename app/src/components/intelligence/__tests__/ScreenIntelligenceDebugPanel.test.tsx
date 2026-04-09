import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { ScreenIntelligenceState } from '../../../features/screen-intelligence/useScreenIntelligenceState';
import { useScreenIntelligenceState } from '../../../features/screen-intelligence/useScreenIntelligenceState';
import ScreenIntelligenceDebugPanel from '../ScreenIntelligenceDebugPanel';

vi.mock('../../../features/screen-intelligence/useScreenIntelligenceState', () => ({
  useScreenIntelligenceState: vi.fn(),
}));

const baseState: ScreenIntelligenceState = {
  status: {
    platform_supported: true,
    permissions: {
      screen_recording: 'granted',
      accessibility: 'granted',
      input_monitoring: 'granted',
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

describe('ScreenIntelligenceDebugPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(useScreenIntelligenceState).mockReturnValue(baseState);
  });

  it('renders successful capture diagnostics and preview image', async () => {
    const state: ScreenIntelligenceState = {
      ...baseState,
      captureTestResult: {
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
      },
    };

    render(<ScreenIntelligenceDebugPanel state={state} />);

    expect(screen.getByText('Success')).toBeInTheDocument();
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
    const state: ScreenIntelligenceState = {
      ...baseState,
      captureTestResult: {
        ok: false,
        capture_mode: 'fullscreen',
        context: null,
        image_ref: null,
        bytes_estimate: null,
        error: 'screen recording permission is not granted',
        timing_ms: 42,
      },
    };

    render(<ScreenIntelligenceDebugPanel state={state} />);

    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));

    expect(screen.getByText('Failed')).toBeInTheDocument();
    expect(screen.getByText('fullscreen')).toBeInTheDocument();
    expect(screen.getByText('42ms')).toBeInTheDocument();
    expect(screen.getByText('screen recording permission is not granted')).toBeInTheDocument();
    expect(screen.queryByAltText('Capture test result')).not.toBeInTheDocument();
    expect(screen.getByText('Permissions')).toBeInTheDocument();
    expect(state.refreshStatus).toHaveBeenCalledTimes(1);
    expect(state.refreshVision).toHaveBeenCalledWith(5);
  });
});
