import { describe, expect, it } from 'vitest';

import type { AccessibilityStatus } from '../../utils/tauriCommands';
import reducer, {
  clearAccessibilityError,
  fetchAccessibilityStatus,
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
  },
  config: {
    capture_policy: 'hybrid',
    baseline_fps: 1,
    session_ttl_secs: 300,
    panic_stop_hotkey: 'Cmd+Shift+.',
    autocomplete_enabled: true,
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
});
