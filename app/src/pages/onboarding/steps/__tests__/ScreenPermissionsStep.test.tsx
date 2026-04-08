import { configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { PropsWithChildren } from 'react';
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
  openhumanAccessibilityRequestPermission,
  openhumanAccessibilityStatus,
  restartCoreProcess,
} from '../../../../utils/tauriCommands';
import ScreenPermissionsStep from '../ScreenPermissionsStep';

vi.mock('../../../../utils/tauriCommands', async importOriginal => {
  const actual = await importOriginal<typeof import('../../../../utils/tauriCommands')>();
  return {
    ...actual,
    openhumanAccessibilityRequestPermission: vi.fn(),
    openhumanAccessibilityStatus: vi.fn(),
    restartCoreProcess: vi.fn(),
  };
});

const deniedStatus: AccessibilityStatus = {
  platform_supported: true,
  permissions: {
    screen_recording: 'unknown',
    accessibility: 'denied',
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
    denylist: [],
  },
  denylist: [],
  is_context_blocked: false,
  permission_check_process_path: '/tmp/openhuman-core-x86_64-apple-darwin',
};

const grantedStatus: AccessibilityStatus = {
  ...deniedStatus,
  permissions: { ...deniedStatus.permissions, accessibility: 'granted' },
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

function renderStep() {
  const store = createStore();
  const onNext = vi.fn();

  const Wrapper = ({ children }: PropsWithChildren) => (
    <Provider store={store}>
      <MemoryRouter>{children}</MemoryRouter>
    </Provider>
  );

  render(<ScreenPermissionsStep onNext={onNext} />, { wrapper: Wrapper });

  return { store, onNext };
}

describe('ScreenPermissionsStep', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(restartCoreProcess).mockResolvedValue(undefined);
    vi.mocked(openhumanAccessibilityRequestPermission).mockResolvedValue({
      result: deniedStatus.permissions,
      logs: [],
    });
    vi.mocked(openhumanAccessibilityStatus).mockResolvedValue({ result: deniedStatus, logs: [] });
  });

  it('auto-refreshes permissions after returning from System Settings', async () => {
    vi.mocked(openhumanAccessibilityStatus)
      .mockResolvedValueOnce({ result: deniedStatus, logs: [] })
      .mockResolvedValueOnce({ result: deniedStatus, logs: [] })
      .mockResolvedValueOnce({ result: grantedStatus, logs: [] });

    renderStep();

    await screen.findByText('Screen & Accessibility Permissions');

    fireEvent.click(screen.getByRole('button', { name: 'Request Permissions' }));

    expect(await screen.findByText(/OpenHuman will refresh automatically/i)).toBeInTheDocument();

    Object.defineProperty(document, 'visibilityState', { configurable: true, value: 'visible' });
    fireEvent(window, new Event('focus'));

    await waitFor(() => {
      expect(restartCoreProcess).toHaveBeenCalledTimes(1);
    });

    await waitFor(() => {
      expect(screen.getByText('granted')).toBeInTheDocument();
    });
  });
});
