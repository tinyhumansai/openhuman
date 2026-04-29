import { isTauri as coreIsTauri } from '@tauri-apps/api/core';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  ensureNotificationPermission,
  getNotificationPermissionState,
  showNativeNotification,
} from '../../lib/nativeNotifications/tauriBridge';
import OpenhumanLinkModal, { OPENHUMAN_LINK_EVENT } from '../OpenhumanLinkModal';

vi.mock('@tauri-apps/api/core', () => ({ isTauri: vi.fn() }));

vi.mock('../../lib/nativeNotifications/tauriBridge', () => ({
  ensureNotificationPermission: vi.fn(),
  getNotificationPermissionState: vi.fn(),
  showNativeNotification: vi.fn(),
}));

describe('OpenhumanLinkModal notifications test flow', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(getNotificationPermissionState).mockResolvedValue('prompt');
  });

  function openNotificationsModal() {
    act(() => {
      window.dispatchEvent(
        new CustomEvent(OPENHUMAN_LINK_EVENT, { detail: { path: 'settings/notifications' } })
      );
    });
  }

  async function flushAsyncWork() {
    await act(async () => {
      await Promise.resolve();
    });
  }

  it('shows success after permission is granted and native notification send succeeds', async () => {
    vi.mocked(coreIsTauri).mockReturnValue(true);
    vi.mocked(ensureNotificationPermission).mockResolvedValue(true);
    vi.mocked(showNativeNotification).mockResolvedValue({ delivered: true });

    render(<OpenhumanLinkModal />);
    openNotificationsModal();

    fireEvent.click(screen.getByRole('button', { name: 'Send test notification' }));
    await flushAsyncWork();

    expect(
      screen.getByText(/Test notification sent\. If you didn’t receive it/i)
    ).toBeInTheDocument();
    expect(showNativeNotification).toHaveBeenCalledWith(
      expect.objectContaining({ tag: 'welcome-notification-test' })
    );
  });

  it('shows actionable error when permission is denied and does not send notification', async () => {
    vi.mocked(coreIsTauri).mockReturnValue(true);
    vi.mocked(ensureNotificationPermission).mockResolvedValue(false);
    vi.mocked(getNotificationPermissionState).mockResolvedValue('denied');

    render(<OpenhumanLinkModal />);
    openNotificationsModal();

    fireEvent.click(screen.getByRole('button', { name: 'Send test notification' }));
    await flushAsyncWork();

    expect(screen.getByText(/Notification permission is off\./i)).toBeInTheDocument();
    expect(showNativeNotification).not.toHaveBeenCalled();
    expect(screen.getByRole('button', { name: 'Retry test notification' })).toBeInTheDocument();
  });

  it('shows send failure message when native notification call fails', async () => {
    vi.mocked(coreIsTauri).mockReturnValue(true);
    vi.mocked(ensureNotificationPermission).mockResolvedValue(true);
    vi.mocked(showNativeNotification).mockResolvedValue({
      delivered: false,
      reason: 'send_failed',
      error: 'notification show failed: test error',
    });

    render(<OpenhumanLinkModal />);
    openNotificationsModal();

    fireEvent.click(screen.getByRole('button', { name: 'Send test notification' }));
    await flushAsyncWork();

    expect(
      screen.getByText(/Couldn't send: notification show failed: test error/i)
    ).toBeInTheDocument();
  });

  it('retries successfully after user grants permission on a second attempt', async () => {
    vi.mocked(coreIsTauri).mockReturnValue(true);
    vi.mocked(ensureNotificationPermission)
      .mockResolvedValueOnce(false)
      .mockResolvedValueOnce(true);
    vi.mocked(getNotificationPermissionState)
      .mockResolvedValueOnce('denied')
      .mockResolvedValueOnce('denied')
      .mockResolvedValueOnce('granted');
    vi.mocked(showNativeNotification).mockResolvedValue({ delivered: true });

    render(<OpenhumanLinkModal />);
    openNotificationsModal();

    fireEvent.click(screen.getByRole('button', { name: 'Send test notification' }));
    await flushAsyncWork();

    expect(screen.getByText(/Notification permission is off\./i)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Retry test notification' }));
    await flushAsyncWork();

    expect(
      screen.getByText(/Test notification sent\. If you didn’t receive it/i)
    ).toBeInTheDocument();
    expect(showNativeNotification).toHaveBeenCalledTimes(1);
  });
});
