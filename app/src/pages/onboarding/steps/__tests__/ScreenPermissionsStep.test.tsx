import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { ScreenIntelligenceState } from '../../../../features/screen-intelligence/useScreenIntelligenceState';
import { useScreenIntelligenceState } from '../../../../features/screen-intelligence/useScreenIntelligenceState';
import ScreenPermissionsStep from '../ScreenPermissionsStep';

vi.mock('../../../../features/screen-intelligence/useScreenIntelligenceState', () => ({
  useScreenIntelligenceState: vi.fn(),
}));

const deniedState: ScreenIntelligenceState = {
  status: {
    platform_supported: true,
    core_process: {
      pid: 4242,
      started_at_ms: 1712700000000,
    },
    permissions: {
      screen_recording: 'unknown',
      accessibility: 'denied',
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
    permission_check_process_path: '/tmp/openhuman-core-x86_64-apple-darwin',
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

describe('ScreenPermissionsStep', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(useScreenIntelligenceState).mockReturnValue(deniedState);
  });

  it('auto-refreshes permissions after returning from System Settings', async () => {
    const onNext = vi.fn();

    render(
      <MemoryRouter>
        <ScreenPermissionsStep onNext={onNext} />
      </MemoryRouter>
    );

    await screen.findByText('Screen & Accessibility Permissions');

    fireEvent.click(screen.getByRole('button', { name: 'Request Permissions' }));

    expect(await screen.findByText(/OpenHuman will refresh automatically/i)).toBeInTheDocument();

    Object.defineProperty(document, 'visibilityState', { configurable: true, value: 'visible' });
    fireEvent(window, new Event('focus'));

    await waitFor(() => {
      expect(deniedState.refreshPermissionsWithRestart).toHaveBeenCalledTimes(1);
    });
  });
});
