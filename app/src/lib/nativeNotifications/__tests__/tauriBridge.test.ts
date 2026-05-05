import { invoke, isTauri } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  ensureNotificationPermission,
  getNotificationPermissionState,
  showNativeNotification,
} from '../tauriBridge';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), isTauri: vi.fn() }));

const mockInvoke = vi.mocked(invoke);
const mockIsTauri = vi.mocked(isTauri);

beforeEach(() => {
  vi.clearAllMocks();
  mockIsTauri.mockReturnValue(true);
});

describe('getNotificationPermissionState', () => {
  it('returns "not_tauri" when not in Tauri runtime', async () => {
    mockIsTauri.mockReturnValue(false);
    await expect(getNotificationPermissionState()).resolves.toBe('not_tauri');
    expect(mockInvoke).not.toHaveBeenCalled();
  });

  it('invokes the dedicated notification_permission_state command, not the plugin command', async () => {
    mockInvoke.mockResolvedValueOnce('granted');
    await getNotificationPermissionState({ requestIfNeeded: false });
    expect(mockInvoke).toHaveBeenCalledWith('notification_permission_state');
    // Regression guard for #1152: the bundled tauri-plugin-notification's
    // permission_state is hardcoded to Granted, so the bridge MUST NOT
    // route through plugin:notification|* for the permission gate.
    expect(mockInvoke).not.toHaveBeenCalledWith('plugin:notification|is_permission_granted');
  });

  it('maps "granted" / "provisional" / "ephemeral" to granted', async () => {
    for (const raw of ['granted', 'provisional', 'ephemeral']) {
      mockInvoke.mockReset();
      mockInvoke.mockResolvedValueOnce(raw);
      await expect(getNotificationPermissionState({ requestIfNeeded: false })).resolves.toBe(
        'granted'
      );
    }
  });

  it('maps "denied" to denied without prompting', async () => {
    mockInvoke.mockResolvedValueOnce('denied');
    await expect(getNotificationPermissionState({ requestIfNeeded: true })).resolves.toBe('denied');
    expect(mockInvoke).toHaveBeenCalledTimes(1);
    expect(mockInvoke).toHaveBeenCalledWith('notification_permission_state');
  });

  it('falls through to notification_permission_request when state is not_determined and requestIfNeeded is true', async () => {
    mockInvoke.mockResolvedValueOnce('not_determined').mockResolvedValueOnce('granted');
    await expect(getNotificationPermissionState({ requestIfNeeded: true })).resolves.toBe(
      'granted'
    );
    expect(mockInvoke).toHaveBeenNthCalledWith(1, 'notification_permission_state');
    expect(mockInvoke).toHaveBeenNthCalledWith(2, 'notification_permission_request');
  });

  it('returns prompt without prompting when requestIfNeeded=false and state is not_determined', async () => {
    mockInvoke.mockResolvedValueOnce('not_determined');
    await expect(getNotificationPermissionState({ requestIfNeeded: false })).resolves.toBe(
      'prompt'
    );
    expect(mockInvoke).toHaveBeenCalledTimes(1);
  });

  it('returns "denied" when the permission prompt is dismissed', async () => {
    mockInvoke.mockResolvedValueOnce('not_determined').mockResolvedValueOnce('denied');
    await expect(getNotificationPermissionState()).resolves.toBe('denied');
  });

  it('returns "unknown" when the underlying invoke throws', async () => {
    mockInvoke.mockRejectedValueOnce(new Error('boom'));
    await expect(getNotificationPermissionState()).resolves.toBe('unknown');
  });
});

describe('ensureNotificationPermission', () => {
  it('returns true only when state resolves to granted', async () => {
    mockInvoke.mockResolvedValueOnce('granted');
    await expect(ensureNotificationPermission()).resolves.toBe(true);
  });

  it('returns false when state resolves to denied', async () => {
    mockInvoke.mockResolvedValueOnce('denied');
    await expect(ensureNotificationPermission()).resolves.toBe(false);
  });
});

describe('showNativeNotification', () => {
  it('returns { delivered: false, reason: "not_tauri" } when not in Tauri', async () => {
    mockIsTauri.mockReturnValue(false);
    await expect(showNativeNotification({ title: 't', body: 'b' })).resolves.toEqual({
      delivered: false,
      reason: 'not_tauri',
    });
    expect(mockInvoke).not.toHaveBeenCalled();
  });

  it('invokes show_native_notification (not plugin:notification|notify) with title/body/tag', async () => {
    mockInvoke.mockResolvedValueOnce(undefined);
    await showNativeNotification({ title: 'Hi', body: 'There', tag: 'welcome' });
    expect(mockInvoke).toHaveBeenCalledWith('show_native_notification', {
      title: 'Hi',
      body: 'There',
      tag: 'welcome',
    });
    expect(mockInvoke).not.toHaveBeenCalledWith('plugin:notification|notify', expect.anything());
  });

  it('passes tag=null when caller omits a tag', async () => {
    mockInvoke.mockResolvedValueOnce(undefined);
    await showNativeNotification({ title: 'Hi', body: '' });
    expect(mockInvoke).toHaveBeenCalledWith('show_native_notification', {
      title: 'Hi',
      body: '',
      tag: null,
    });
  });

  it('returns delivered:true on success', async () => {
    mockInvoke.mockResolvedValueOnce(undefined);
    await expect(showNativeNotification({ title: 't', body: 'b' })).resolves.toEqual({
      delivered: true,
    });
  });

  it('surfaces the Rust error string as { delivered:false, reason:"send_failed", error }', async () => {
    mockInvoke.mockRejectedValueOnce(
      new Error('notification permission not granted (state: denied)')
    );
    await expect(showNativeNotification({ title: 't', body: 'b' })).resolves.toEqual({
      delivered: false,
      reason: 'send_failed',
      error: 'notification permission not granted (state: denied)',
    });
  });

  it('coerces non-Error rejections to a string error', async () => {
    mockInvoke.mockRejectedValueOnce('plain string');
    await expect(showNativeNotification({ title: 't', body: 'b' })).resolves.toEqual({
      delivered: false,
      reason: 'send_failed',
      error: 'plain string',
    });
  });
});
