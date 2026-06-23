import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, test, vi } from 'vitest';

import ErrorFallbackScreen from './ErrorFallbackScreen';

// `t` echoes the key so assertions can target stable key strings.
vi.mock('../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

const hoisted = vi.hoisted(() => ({
  openUrl: vi.fn(),
  safeInvoke: vi.fn().mockResolvedValue(undefined),
  isTauri: vi.fn(() => true),
}));

vi.mock('../utils/openUrl', () => ({ openUrl: hoisted.openUrl }));
vi.mock('../utils/tauriCommands/common', () => ({
  safeInvoke: hoisted.safeInvoke,
  isTauri: hoisted.isTauri,
}));
vi.mock('../utils/config', () => ({
  SUPPORT_URL: 'https://support.example/help',
  LATEST_APP_DOWNLOAD_URL: 'https://downloads.example/latest',
}));

const baseProps = {
  error: new Error('boom went the render'),
  componentStack: '    at Foo\n    at Bar',
  onReset: vi.fn(),
};

afterEach(() => {
  vi.clearAllMocks();
});

describe('ErrorFallbackScreen', () => {
  test('shows a copyable Error ID and copies it on click', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });

    render(<ErrorFallbackScreen {...baseProps} eventId="abc123def456" />);

    expect(screen.getByText('app.errorFallback.eventIdLabel')).toBeInTheDocument();
    expect(screen.getByText('abc123def456')).toBeInTheDocument();

    fireEvent.click(screen.getByText('app.errorFallback.copyEventId'));
    expect(writeText).toHaveBeenCalledWith('abc123def456');
    await waitFor(() =>
      expect(screen.getByText('app.errorFallback.eventIdCopied')).toBeInTheDocument()
    );
  });

  test('hides the Error ID and support link when no event id', () => {
    render(<ErrorFallbackScreen {...baseProps} eventId={null} />);
    expect(screen.queryByText('app.errorFallback.eventIdLabel')).not.toBeInTheDocument();
    expect(screen.queryByText('app.errorFallback.contactSupport')).not.toBeInTheDocument();
  });

  test('opens the support deep link seeded with the (encoded) event id', () => {
    render(<ErrorFallbackScreen {...baseProps} eventId="id/with space" />);
    fireEvent.click(screen.getByText('app.errorFallback.contactSupport'));
    expect(hoisted.openUrl).toHaveBeenCalledWith(
      'https://support.example/help?ref=id%2Fwith%20space'
    );
  });

  test('reveals the logs folder via the tauri command', () => {
    render(<ErrorFallbackScreen {...baseProps} eventId="x" />);
    fireEvent.click(screen.getByText('app.errorFallback.revealLogs'));
    expect(hoisted.safeInvoke).toHaveBeenCalledWith('reveal_logs_folder');
  });

  test('hides Reveal logs outside Tauri', () => {
    hoisted.isTauri.mockReturnValueOnce(false);
    render(<ErrorFallbackScreen {...baseProps} eventId="x" />);
    expect(screen.queryByText('app.errorFallback.revealLogs')).not.toBeInTheDocument();
  });

  test('Try recover calls onReset', () => {
    const onReset = vi.fn();
    render(<ErrorFallbackScreen {...baseProps} onReset={onReset} eventId="x" />);
    fireEvent.click(screen.getByText('app.errorFallback.tryRecover'));
    expect(onReset).toHaveBeenCalledTimes(1);
  });

  test('Download latest opens the release page', () => {
    render(<ErrorFallbackScreen {...baseProps} eventId={null} />);
    fireEvent.click(screen.getByText('app.errorFallback.downloadLatest'));
    expect(hoisted.openUrl).toHaveBeenCalledWith('https://downloads.example/latest');
  });
});
