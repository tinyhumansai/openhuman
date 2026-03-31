import { describe, expect, it } from 'vitest';

import type { AccessibilityStatus, CaptureTestResult } from '../../utils/tauriCommands';
import reducer, {
  clearAccessibilityError,
  fetchAccessibilityStatus,
  runCaptureTest,
  setAccessibilityStatus,
  startAccessibilitySession,
  stopAccessibilitySession,
} from '../accessibilitySlice';

const sampleStatus: AccessibilityStatus = {
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
    enabled: true,
    capture_policy: 'hybrid',
    policy_mode: 'all_except_blacklist',
    baseline_fps: 1,
    vision_enabled: true,
    session_ttl_secs: 300,
    panic_stop_hotkey: 'Cmd+Shift+.',
    autocomplete_enabled: true,
    allowlist: [],
    denylist: ['wallet'],
  },
  denylist: ['wallet'],
  is_context_blocked: false,
};

describe('accessibilitySlice', () => {
  it('has expected initial state', () => {
    const state = reducer(undefined, { type: '@@INIT' });
    expect(state.status).toBeNull();
    expect(state.isLoading).toBe(false);
    expect(state.lastError).toBeNull();
  });

  it('stores status payload', () => {
    const state = reducer(undefined, setAccessibilityStatus(sampleStatus));
    expect(state.status?.platform_supported).toBe(true);
    expect(state.status?.config.capture_policy).toBe('hybrid');
  });

  it('tracks fetch lifecycle', () => {
    const pending = reducer(undefined, { type: fetchAccessibilityStatus.pending.type });
    expect(pending.isLoading).toBe(true);

    const fulfilled = reducer(
      pending,
      fetchAccessibilityStatus.fulfilled(sampleStatus, 'req-1', undefined)
    );
    expect(fulfilled.isLoading).toBe(false);
    expect(fulfilled.status?.permissions.accessibility).toBe('granted');
  });

  it('tracks session start/stop async flags', () => {
    const starting = reducer(undefined, { type: startAccessibilitySession.pending.type });
    expect(starting.isStartingSession).toBe(true);

    const started = reducer(
      starting,
      startAccessibilitySession.fulfilled(sampleStatus, 'req-2', { consent: true })
    );
    expect(started.isStartingSession).toBe(false);

    const stopping = reducer(started, { type: stopAccessibilitySession.pending.type });
    expect(stopping.isStoppingSession).toBe(true);
  });

  it('clears errors', () => {
    const errored = reducer(undefined, {
      type: fetchAccessibilityStatus.rejected.type,
      payload: 'boom',
    });
    expect(errored.lastError).toBe('boom');

    const cleared = reducer(errored, clearAccessibilityError());
    expect(cleared.lastError).toBeNull();
  });

  it('tracks capture test lifecycle', () => {
    const pending = reducer(undefined, { type: runCaptureTest.pending.type });
    expect(pending.isCaptureTestRunning).toBe(true);
    expect(pending.captureTestResult).toBeNull();

    const testResult: CaptureTestResult = {
      ok: true,
      capture_mode: 'windowed',
      context: {
        app_name: 'Safari',
        window_title: 'GitHub',
        bounds_x: 0,
        bounds_y: 0,
        bounds_width: 1400,
        bounds_height: 900,
      },
      image_ref: 'data:image/png;base64,abc',
      bytes_estimate: 12345,
      error: null,
      timing_ms: 150,
    };

    const fulfilled = reducer(pending, runCaptureTest.fulfilled(testResult, 'req-3', undefined));
    expect(fulfilled.isCaptureTestRunning).toBe(false);
    expect(fulfilled.captureTestResult?.ok).toBe(true);
    expect(fulfilled.captureTestResult?.capture_mode).toBe('windowed');
  });

  it('handles capture test failure', () => {
    const rejected = reducer(undefined, {
      type: runCaptureTest.rejected.type,
      payload: 'capture failed',
    });
    expect(rejected.isCaptureTestRunning).toBe(false);
    expect(rejected.lastError).toBe('capture failed');
  });
});
