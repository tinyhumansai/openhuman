import { render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';

import type { ScreenIntelligenceState } from '../../../../features/screen-intelligence/useScreenIntelligenceState';
import AccessibilityPanel from '../AccessibilityPanel';

vi.mock('../../../../features/screen-intelligence/useScreenIntelligenceState', () => ({
  useScreenIntelligenceState: vi.fn(),
}));

import { useScreenIntelligenceState } from '../../../../features/screen-intelligence/useScreenIntelligenceState';

const mockState: ScreenIntelligenceState = {
  status: {
    platform_supported: true,
    permissions: {
      screen_recording: 'unknown',
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
      denylist: ['wallet'],
    },
    denylist: ['wallet'],
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
  refreshStatus: vi.fn(),
  requestPermission: vi.fn(),
  refreshPermissionsWithRestart: vi.fn(),
  startSession: vi.fn(),
  stopSession: vi.fn(),
  refreshVision: vi.fn(),
  flushVision: vi.fn(),
  runCaptureTest: vi.fn(),
  clearError: vi.fn(),
};

describe('AccessibilityPanel', () => {
  it('renders permission and session sections', () => {
    vi.mocked(useScreenIntelligenceState).mockReturnValue(mockState);

    render(
      <MemoryRouter initialEntries={['/settings/accessibility']}>
        <AccessibilityPanel />
      </MemoryRouter>
    );

    expect(screen.getByText('Accessibility Automation')).toBeInTheDocument();
    expect(screen.getByText('Permissions')).toBeInTheDocument();
    expect(screen.getByText('Session')).toBeInTheDocument();
    expect(screen.queryByText('Screen Recording')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Start Session' })).toBeInTheDocument();
  });
});
