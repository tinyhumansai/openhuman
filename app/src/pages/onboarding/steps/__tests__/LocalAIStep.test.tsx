import { fireEvent, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import LocalAIStep from '../LocalAIStep';

vi.mock('../../../../utils/tauriCommands', () => ({
  openhumanLocalAiDownload: vi.fn().mockResolvedValue({} as never),
  openhumanLocalAiDownloadAllAssets: vi.fn().mockResolvedValue({} as never),
}));

describe('LocalAIStep', () => {
  it('happy path: advances immediately and calls onNext with correct payload', async () => {
    const onNext = vi.fn();
    renderWithProviders(<LocalAIStep onNext={onNext} />);

    fireEvent.click(screen.getByRole('button', { name: /use local models/i }));

    expect(onNext).toHaveBeenCalledOnce();
    expect(onNext).toHaveBeenCalledWith({ consentGiven: true, downloadStarted: true });
  });

  it('error path: calls onDownloadError once when openhumanLocalAiDownload rejects', async () => {
    const { openhumanLocalAiDownload } = await import('../../../../utils/tauriCommands');
    vi.mocked(openhumanLocalAiDownload).mockRejectedValueOnce(new Error('network error'));

    const onNext = vi.fn();
    const onDownloadError = vi.fn();
    renderWithProviders(<LocalAIStep onNext={onNext} onDownloadError={onDownloadError} />);

    fireEvent.click(screen.getByRole('button', { name: /use local models/i }));

    // onNext still fires immediately
    expect(onNext).toHaveBeenCalledOnce();

    // onDownloadError fires asynchronously after the rejected promise settles
    await waitFor(() => {
      expect(onDownloadError).toHaveBeenCalledOnce();
    });
    expect(onDownloadError).toHaveBeenCalledWith('Local AI setup encountered an issue');
  });

  it('error path: calls onDownloadError only once even if both downloads fail', async () => {
    const { openhumanLocalAiDownload, openhumanLocalAiDownloadAllAssets } =
      await import('../../../../utils/tauriCommands');
    vi.mocked(openhumanLocalAiDownload).mockRejectedValueOnce(new Error('fail 1'));
    vi.mocked(openhumanLocalAiDownloadAllAssets).mockRejectedValueOnce(new Error('fail 2'));

    const onNext = vi.fn();
    const onDownloadError = vi.fn();
    renderWithProviders(<LocalAIStep onNext={onNext} onDownloadError={onDownloadError} />);

    fireEvent.click(screen.getByRole('button', { name: /use local models/i }));

    await waitFor(() => {
      expect(onDownloadError).toHaveBeenCalledOnce();
    });
  });

  it('double-click guard: download functions called only once', async () => {
    const { openhumanLocalAiDownload, openhumanLocalAiDownloadAllAssets } =
      await import('../../../../utils/tauriCommands');
    vi.mocked(openhumanLocalAiDownload).mockResolvedValue({} as never);
    vi.mocked(openhumanLocalAiDownloadAllAssets).mockResolvedValue({} as never);

    const onNext = vi.fn();
    renderWithProviders(<LocalAIStep onNext={onNext} />);

    const button = screen.getByRole('button', { name: /use local models/i });
    fireEvent.click(button);
    fireEvent.click(button);

    expect(onNext).toHaveBeenCalledOnce();
    expect(openhumanLocalAiDownload).toHaveBeenCalledOnce();
    expect(openhumanLocalAiDownloadAllAssets).toHaveBeenCalledOnce();
  });
});
