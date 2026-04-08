import { configureStore } from '@reduxjs/toolkit';
import { render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';
import { describe, expect, it } from 'vitest';

import accessibilityReducer from '../../../../store/accessibilitySlice';
import authReducer from '../../../../store/authSlice';
import socketReducer from '../../../../store/socketSlice';
import teamReducer from '../../../../store/teamSlice';
import userReducer from '../../../../store/userSlice';
import type { AccessibilityStatus } from '../../../../utils/tauriCommands';
import AccessibilityPanel from '../AccessibilityPanel';

const status: AccessibilityStatus = {
  platform_supported: true,
  permissions: {
    screen_recording: 'unknown',
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
    preloadedState: {
      accessibility: {
        status,
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
      },
    },
  });

describe('AccessibilityPanel', () => {
  it('renders permission and session sections', () => {
    const store = createStore();

    render(
      <Provider store={store}>
        <MemoryRouter initialEntries={['/settings/accessibility']}>
          <AccessibilityPanel />
        </MemoryRouter>
      </Provider>
    );

    expect(screen.getByText('Accessibility Automation')).toBeInTheDocument();
    expect(screen.getByText('Permissions')).toBeInTheDocument();
    expect(screen.getByText('Session')).toBeInTheDocument();
    expect(screen.queryByText('Screen Recording')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Start Session' })).toBeInTheDocument();
  });
});
