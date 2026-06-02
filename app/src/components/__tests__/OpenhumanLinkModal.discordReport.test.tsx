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

// Clipboard mock — set up per-test
const mockWriteText = vi.fn();

describe('OpenhumanLinkModal discord-report flow', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // Install clipboard mock onto navigator
    Object.defineProperty(navigator, 'clipboard', {
      value: { writeText: mockWriteText },
      configurable: true,
      writable: true,
    });
    mockWriteText.mockResolvedValue(undefined);
  });

  function openReportModal(context?: string) {
    act(() => {
      window.dispatchEvent(
        new CustomEvent(OPENHUMAN_LINK_EVENT, {
          detail: { path: 'community/discord-report', context },
        })
      );
    });
  }

  it('dispatching the event with discord-report path opens the modal with the report title', () => {
    render(<OpenhumanLinkModal />);
    openReportModal('boom detail');

    // The report title (not the join-community title) should be visible
    expect(screen.getByText('Report this error')).toBeInTheDocument();
    // The generic join-community title must NOT appear
    expect(screen.queryByText('Join the community')).not.toBeInTheDocument();
  });

  it('clicking "Copy error details" calls clipboard.writeText with the provided context', async () => {
    render(<OpenhumanLinkModal />);
    openReportModal('boom detail');

    fireEvent.click(screen.getByRole('button', { name: 'Copy error details' }));

    await waitFor(() => {
      expect(mockWriteText).toHaveBeenCalledWith('boom detail');
    });
  });

  it('button label switches to "Copied to clipboard" after a successful copy', async () => {
    render(<OpenhumanLinkModal />);
    openReportModal('boom detail');

    fireEvent.click(screen.getByRole('button', { name: 'Copy error details' }));

    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Copied to clipboard' })).toBeInTheDocument();
    });
  });

  it('with no context, copy writes the fallback string', async () => {
    render(<OpenhumanLinkModal />);
    // Dispatch without context
    openReportModal(undefined);

    fireEvent.click(screen.getByRole('button', { name: 'Copy error details' }));

    await waitFor(() => {
      expect(mockWriteText).toHaveBeenCalledWith(
        'OpenHuman agent error (no additional details were available).'
      );
    });
  });

  it('with empty-string context, copy writes the fallback string', async () => {
    render(<OpenhumanLinkModal />);
    openReportModal('   ');

    fireEvent.click(screen.getByRole('button', { name: 'Copy error details' }));

    await waitFor(() => {
      expect(mockWriteText).toHaveBeenCalledWith(
        'OpenHuman agent error (no additional details were available).'
      );
    });
  });

  it('clicking "Open Discord" calls openUrl with the Discord invite URL', () => {
    const openUrlSpy = vi.spyOn(openUrlModule, 'openUrl').mockResolvedValue(undefined);

    render(<OpenhumanLinkModal />);
    openReportModal('some context');

    fireEvent.click(screen.getByRole('button', { name: 'Open Discord' }));

    expect(openUrlSpy).toHaveBeenCalledWith('https://discord.tinyhumans.ai/');
  });
});
