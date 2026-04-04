import { screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import LocalAIDownloadSnackbar from '../LocalAIDownloadSnackbar';

// Default: isTauri returns false, so snackbar should not render
vi.mock('../../utils/tauriCommands', () => ({
  isTauri: vi.fn(() => false),
  openhumanLocalAiStatus: vi.fn().mockResolvedValue({ result: null }),
  openhumanLocalAiDownloadsProgress: vi.fn().mockResolvedValue({ result: null }),
}));

describe('LocalAIDownloadSnackbar', () => {
  it('does not render when not in Tauri environment', () => {
    renderWithProviders(<LocalAIDownloadSnackbar />);

    expect(screen.queryByText('Downloading')).not.toBeInTheDocument();
    expect(screen.queryByLabelText('Dismiss download notification')).not.toBeInTheDocument();
  });

  it('does not render when no download is active', async () => {
    const tauriCommands = await import('../../utils/tauriCommands');
    vi.mocked(tauriCommands.isTauri).mockReturnValue(true);
    vi.mocked(tauriCommands.openhumanLocalAiStatus).mockResolvedValue({
      result: { state: 'ready' } as never,
      logs: [],
    });
    vi.mocked(tauriCommands.openhumanLocalAiDownloadsProgress).mockResolvedValue({
      result: { state: 'idle', progress: null } as never,
      logs: [],
    });

    renderWithProviders(<LocalAIDownloadSnackbar />);

    // Wait for poll cycle
    await vi.waitFor(() => {
      expect(screen.queryByText('Downloading')).not.toBeInTheDocument();
    });

    // Reset mock
    vi.mocked(tauriCommands.isTauri).mockReturnValue(false);
  });

  it('renders immediately when status reports bootstrap activity before downloads progress catches up', async () => {
    const tauriCommands = await import('../../utils/tauriCommands');
    vi.mocked(tauriCommands.isTauri).mockReturnValue(true);
    vi.mocked(tauriCommands.openhumanLocalAiStatus).mockResolvedValue({
      result: {
        state: 'loading',
        download_progress: 0.42,
        downloaded_bytes: 512 * 1024 * 1024,
        total_bytes: 1024 * 1024 * 1024,
        warning: 'Connecting to local Ollama runtime',
      } as never,
      logs: [],
    });
    vi.mocked(tauriCommands.openhumanLocalAiDownloadsProgress).mockResolvedValue({
      result: { state: 'idle', progress: null } as never,
      logs: [],
    });

    renderWithProviders(<LocalAIDownloadSnackbar />);

    await vi.waitFor(() => {
      expect(screen.getByText('Loading model...')).toBeInTheDocument();
      expect(screen.getByText('512 MB / 1.0 GB')).toBeInTheDocument();
    });

    vi.mocked(tauriCommands.isTauri).mockReturnValue(false);
  });
});
