import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import ServiceBlockingGate from '../ServiceBlockingGate';

const mockOpenUrl = vi.fn();
const mockUseCoreState = vi.fn();
const mockUseDaemonHealth = vi.fn();
const mockUseDaemonLifecycle = vi.fn();

vi.mock('../../../utils/openUrl', () => ({
  openUrl: (...args: unknown[]) => mockOpenUrl(...args),
}));

vi.mock('../../../utils/config', () => ({
  LATEST_APP_DOWNLOAD_URL: 'https://github.com/tinyhumansai/openhuman/releases/latest',
}));

vi.mock('../../../providers/CoreStateProvider', () => ({ useCoreState: () => mockUseCoreState() }));

vi.mock('../../../hooks/useDaemonHealth', () => ({ useDaemonHealth: () => mockUseDaemonHealth() }));

vi.mock('../../../hooks/useDaemonLifecycle', () => ({
  useDaemonLifecycle: () => mockUseDaemonLifecycle(),
}));

describe('ServiceBlockingGate', () => {
  beforeEach(() => {
    mockOpenUrl.mockReset();
    mockUseCoreState.mockReturnValue({ snapshot: { sessionToken: 'token' } });
    mockUseDaemonHealth.mockReturnValue({ status: 'running', restartDaemon: vi.fn() });
    mockUseDaemonLifecycle.mockReturnValue({ maxAttemptsReached: false, resetRetries: vi.fn() });
  });

  it('renders children and does not show recovery prompt when daemon is healthy', async () => {
    render(
      <ServiceBlockingGate>
        <div>App Content</div>
      </ServiceBlockingGate>
    );

    await waitFor(() => expect(screen.getByText('App Content')).toBeInTheDocument());
    expect(screen.queryByText('OpenHuman core is unavailable')).not.toBeInTheDocument();
  });

  it('shows recovery prompt when daemon retries are exhausted', async () => {
    mockUseDaemonHealth.mockReturnValue({ status: 'error', restartDaemon: vi.fn() });
    mockUseDaemonLifecycle.mockReturnValue({ maxAttemptsReached: true, resetRetries: vi.fn() });

    render(
      <ServiceBlockingGate>
        <div>App Content</div>
      </ServiceBlockingGate>
    );

    expect(screen.getByText('OpenHuman core is unavailable')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Download Latest Version' })).toBeInTheDocument();
  });

  it('opens latest release page from recovery prompt', async () => {
    mockUseDaemonHealth.mockReturnValue({ status: 'error', restartDaemon: vi.fn() });
    mockUseDaemonLifecycle.mockReturnValue({ maxAttemptsReached: true, resetRetries: vi.fn() });

    render(
      <ServiceBlockingGate>
        <div>App Content</div>
      </ServiceBlockingGate>
    );

    fireEvent.click(screen.getByRole('button', { name: 'Download Latest Version' }));

    await waitFor(() => {
      expect(mockOpenUrl).toHaveBeenCalledWith(
        'https://github.com/tinyhumansai/openhuman/releases/latest'
      );
    });
  });

  it('calls restartDaemon and resetRetries on successful retry', async () => {
    const mockRestartDaemon = vi.fn().mockResolvedValue(true);
    const mockResetRetries = vi.fn();
    mockUseDaemonHealth.mockReturnValue({ status: 'error', restartDaemon: mockRestartDaemon });
    mockUseDaemonLifecycle.mockReturnValue({
      maxAttemptsReached: true,
      resetRetries: mockResetRetries,
    });

    render(
      <ServiceBlockingGate>
        <div>App Content</div>
      </ServiceBlockingGate>
    );

    fireEvent.click(screen.getByRole('button', { name: 'Retry Core' }));

    await waitFor(() => {
      expect(mockRestartDaemon).toHaveBeenCalled();
      expect(mockResetRetries).toHaveBeenCalled();
    });
  });

  it('shows error message when restartDaemon returns false', async () => {
    const mockRestartDaemon = vi.fn().mockResolvedValue(false);
    mockUseDaemonHealth.mockReturnValue({ status: 'error', restartDaemon: mockRestartDaemon });
    mockUseDaemonLifecycle.mockReturnValue({ maxAttemptsReached: true, resetRetries: vi.fn() });

    render(
      <ServiceBlockingGate>
        <div>App Content</div>
      </ServiceBlockingGate>
    );

    fireEvent.click(screen.getByRole('button', { name: 'Retry Core' }));

    await waitFor(() => {
      expect(screen.getByText(/Retry failed/)).toBeInTheDocument();
    });
  });

  it('shows error message when restartDaemon throws', async () => {
    const mockRestartDaemon = vi.fn().mockRejectedValue(new Error('daemon error'));
    mockUseDaemonHealth.mockReturnValue({ status: 'error', restartDaemon: mockRestartDaemon });
    mockUseDaemonLifecycle.mockReturnValue({ maxAttemptsReached: true, resetRetries: vi.fn() });

    render(
      <ServiceBlockingGate>
        <div>App Content</div>
      </ServiceBlockingGate>
    );

    fireEvent.click(screen.getByRole('button', { name: 'Retry Core' }));

    await waitFor(() => {
      expect(screen.getByText(/Retry failed/)).toBeInTheDocument();
    });
  });
});
