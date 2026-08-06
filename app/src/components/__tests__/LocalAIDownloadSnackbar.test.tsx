import { screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { getCoreStateSnapshot, setCoreStateSnapshot } from '../../lib/coreState/store';
import { renderWithProviders } from '../../test/test-utils';
import LocalAIDownloadSnackbar from '../LocalAIDownloadSnackbar';

// Default: isTauri returns false, so snackbar should not render
vi.mock('../../utils/tauriCommands', () => ({
  isTauri: vi.fn(() => false),
  openhumanLocalAiStatus: vi.fn().mockResolvedValue({ result: null }),
  openhumanLocalAiDownloadsProgress: vi.fn().mockResolvedValue({ result: null }),
}));

/**
 * Seed the folded `runtime.localAi.state` in core state. The snackbar reads this
 * (instead of an idle inference poll) to decide whether to run its fast poll, so
 * tests that expect it to poll must mark a download active here first.
 */
function seedLocalAiState(state: string | null): void {
  const current = getCoreStateSnapshot();
  setCoreStateSnapshot({
    ...current,
    snapshot: {
      ...current.snapshot,
      runtime: {
        ...current.snapshot.runtime,
        localAi: state === null ? null : ({ state } as never),
      },
    },
  });
}

afterEach(() => {
  seedLocalAiState(null);
});

describe('LocalAIDownloadSnackbar', () => {
  it('does not render when not in Tauri environment', () => {
    renderWithProviders(<LocalAIDownloadSnackbar />);

    expect(screen.queryByText('Downloading')).not.toBeInTheDocument();
    expect(screen.queryByLabelText('Dismiss download notification')).not.toBeInTheDocument();
  });

  it('does not poll or render when core state reports no active download', async () => {
    const tauriCommands = await import('../../utils/tauriCommands');
    vi.mocked(tauriCommands.isTauri).mockReturnValue(true);
    vi.mocked(tauriCommands.openhumanLocalAiStatus).mockClear();
    vi.mocked(tauriCommands.openhumanLocalAiDownloadsProgress).mockClear();
    seedLocalAiState('ready');

    renderWithProviders(<LocalAIDownloadSnackbar />);

    await vi.waitFor(() => {
      expect(screen.queryByText('Downloading')).not.toBeInTheDocument();
    });
    // Idle → zero inference polls (the fold's whole point).
    expect(tauriCommands.openhumanLocalAiStatus).not.toHaveBeenCalled();
    expect(tauriCommands.openhumanLocalAiDownloadsProgress).not.toHaveBeenCalled();

    vi.mocked(tauriCommands.isTauri).mockReturnValue(false);
  });

  it('polls and renders when core state reports an active download', async () => {
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
    seedLocalAiState('loading');

    renderWithProviders(<LocalAIDownloadSnackbar />);

    await vi.waitFor(() => {
      expect(screen.getByText('Loading model...')).toBeInTheDocument();
      expect(screen.getByText('512 MB / 1.0 GB')).toBeInTheDocument();
    });

    vi.mocked(tauriCommands.isTauri).mockReturnValue(false);
  });
});
