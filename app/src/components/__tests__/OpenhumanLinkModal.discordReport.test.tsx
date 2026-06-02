import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import * as openUrlModule from '../../utils/openUrl';
import OpenhumanLinkModal, { OPENHUMAN_LINK_EVENT } from '../OpenhumanLinkModal';

// Mock modules that require Tauri runtime or browser APIs not in jsdom
vi.mock('../../services/webviewAccountService', () => ({
  isTauri: vi.fn(() => false),
  purgeWebviewAccount: vi.fn(),
}));

vi.mock('../../lib/nativeNotifications/tauriBridge', () => ({
  ensureNotificationPermission: vi.fn(),
  getNotificationPermissionState: vi.fn().mockResolvedValue('prompt'),
  showNativeNotification: vi.fn(),
}));

// Mock openUrl so "Open Discord" tests don't hit the real URL opener
vi.mock('../../utils/openUrl', () => ({ openUrl: vi.fn().mockResolvedValue(undefined) }));

describe('OpenhumanLinkModal discord-report flow', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  function openReportModal() {
    act(() => {
      window.dispatchEvent(
        new CustomEvent(OPENHUMAN_LINK_EVENT, { detail: { path: 'community/discord-report' } })
      );
    });
  }

  it('dispatching the event with discord-report path opens the modal with the report title', () => {
    render(<OpenhumanLinkModal />);
    openReportModal();

    // The report title (not the join-community title) should be visible
    expect(screen.getByText('Report this error')).toBeInTheDocument();
    // The generic join-community title must NOT appear
    expect(screen.queryByText('Join the community')).not.toBeInTheDocument();
  });

  it('clicking "Open Discord" calls openUrl with the Discord invite URL', () => {
    const openUrlSpy = vi.spyOn(openUrlModule, 'openUrl').mockResolvedValue(undefined);

    render(<OpenhumanLinkModal />);
    openReportModal();

    fireEvent.click(screen.getByRole('button', { name: 'Open Discord' }));

    expect(openUrlSpy).toHaveBeenCalledWith('https://discord.tinyhumans.ai');
  });

  it('clicking "Open Discord" closes the modal', async () => {
    render(<OpenhumanLinkModal />);
    openReportModal();

    expect(screen.getByText('Report this error')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Open Discord' }));

    // The handler awaits openUrl before closing, so the close runs on a
    // microtask — wait for the modal to leave the DOM.
    await waitFor(() => {
      expect(screen.queryByText('Report this error')).not.toBeInTheDocument();
    });
  });
});

describe('OpenhumanLinkModal discord join-community flow', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  function openJoinModal() {
    act(() => {
      window.dispatchEvent(
        new CustomEvent(OPENHUMAN_LINK_EVENT, { detail: { path: 'community/discord' } })
      );
    });
  }

  it('opens the join-community modal (not the error-report variant)', () => {
    render(<OpenhumanLinkModal />);
    openJoinModal();

    expect(screen.getByText('Join the community')).toBeInTheDocument();
    expect(screen.queryByText('Report this error')).not.toBeInTheDocument();
  });

  it('clicking "Open invite link" calls openUrl with the shared Discord URL', async () => {
    const openUrlSpy = vi.spyOn(openUrlModule, 'openUrl').mockResolvedValue(undefined);

    render(<OpenhumanLinkModal />);
    openJoinModal();

    fireEvent.click(screen.getByRole('button', { name: 'Open invite link' }));

    await waitFor(() => {
      expect(openUrlSpy).toHaveBeenCalledWith('https://discord.tinyhumans.ai');
    });
  });

  it('swallows openUrl errors without throwing', async () => {
    const openUrlSpy = vi
      .spyOn(openUrlModule, 'openUrl')
      .mockRejectedValue(new Error('launcher failed'));

    render(<OpenhumanLinkModal />);
    openJoinModal();

    fireEvent.click(screen.getByRole('button', { name: 'Open invite link' }));

    await waitFor(() => {
      expect(openUrlSpy).toHaveBeenCalled();
    });
    // The modal stays mounted; the rejection is caught and ignored.
    expect(screen.getByText('Join the community')).toBeInTheDocument();
  });
});
