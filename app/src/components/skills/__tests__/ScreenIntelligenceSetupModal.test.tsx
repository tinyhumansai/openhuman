import { render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  type ScreenIntelligenceState,
  useScreenIntelligenceState,
} from '../../../features/screen-intelligence/useScreenIntelligenceState';
import ScreenIntelligenceSetupModal from '../ScreenIntelligenceSetupModal';

vi.mock('../../../features/screen-intelligence/useScreenIntelligenceState', () => ({
  useScreenIntelligenceState: vi.fn(),
}));

vi.mock('react-router-dom', async () => {
  const actual = await vi.importActual<Record<string, unknown>>('react-router-dom');
  return { ...actual, useNavigate: vi.fn(() => vi.fn()) };
});

vi.mock('../../../utils/tauriCommands', async () => {
  const actual = await vi.importActual<Record<string, unknown>>('../../../utils/tauriCommands');
  return {
    ...actual,
    openhumanUpdateScreenIntelligenceSettings: vi.fn().mockResolvedValue(undefined),
  };
});

const baseState: ScreenIntelligenceState = {
  status: {
    platform_supported: true,
    permissions: {
      screen_recording: 'unknown',
      accessibility: 'unknown',
      input_monitoring: 'unknown',
    },
    features: { screen_monitoring: false },
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

describe('ScreenIntelligenceSetupModal', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('shows macOS-only message and a Close button when platform_supported is false', () => {
    vi.mocked(useScreenIntelligenceState).mockReturnValue({
      ...baseState,
      status: { ...baseState.status!, platform_supported: false },
    });

    render(<ScreenIntelligenceSetupModal onClose={vi.fn()} />);

    expect(screen.getByText(/macOS only/i)).toBeInTheDocument();
    expect(screen.getByText('Close', { selector: 'button' })).toBeInTheDocument();
    expect(screen.queryByText('Grant permissions')).not.toBeInTheDocument();
    expect(screen.queryByText('Screen Recording')).not.toBeInTheDocument();
  });

  it('calls onClose when the Close button is clicked on the unsupported-platform screen', () => {
    const onClose = vi.fn();
    vi.mocked(useScreenIntelligenceState).mockReturnValue({
      ...baseState,
      status: { ...baseState.status!, platform_supported: false },
    });

    render(<ScreenIntelligenceSetupModal onClose={onClose} />);

    screen.getByText('Close', { selector: 'button' }).click();

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('renders the permissions setup flow when platform_supported is true', () => {
    vi.mocked(useScreenIntelligenceState).mockReturnValue(baseState);

    render(<ScreenIntelligenceSetupModal onClose={vi.fn()} />);

    expect(screen.getByText('Grant permissions')).toBeInTheDocument();
    expect(screen.getByText('Screen Recording')).toBeInTheDocument();
    expect(screen.getByText('Accessibility')).toBeInTheDocument();
    expect(screen.getByText('Input Monitoring')).toBeInTheDocument();
    expect(screen.queryByText(/macOS only/i)).not.toBeInTheDocument();
  });
});
