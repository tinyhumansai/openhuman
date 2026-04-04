import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import LocalAIStep from '../LocalAIStep';

vi.mock('../../../../utils/localAiBootstrap', () => ({
  bootstrapLocalAiWithRecommendedPreset: vi.fn().mockResolvedValue({} as never),
}));

describe('LocalAIStep', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('happy path: advances immediately and calls onNext with correct payload', async () => {
    const onNext = vi.fn();
    renderWithProviders(<LocalAIStep onNext={onNext} />);

    fireEvent.click(screen.getByRole('button', { name: /continue/i }));

    expect(onNext).toHaveBeenCalledOnce();
    expect(onNext).toHaveBeenCalledWith({ consentGiven: true, downloadStarted: true });
  });

  it('error path: calls onDownloadError once when bootstrap fails', async () => {
    const { bootstrapLocalAiWithRecommendedPreset } =
      await import('../../../../utils/localAiBootstrap');
    vi.mocked(bootstrapLocalAiWithRecommendedPreset).mockRejectedValueOnce(
      new Error('network error')
    );

    const onNext = vi.fn();
    const onDownloadError = vi.fn();
    renderWithProviders(<LocalAIStep onNext={onNext} onDownloadError={onDownloadError} />);

    fireEvent.click(screen.getByRole('button', { name: /continue/i }));

    // onNext still fires immediately
    expect(onNext).toHaveBeenCalledOnce();

    // onDownloadError fires asynchronously after the rejected promise settles
    await waitFor(() => {
      expect(onDownloadError).toHaveBeenCalledOnce();
    });
    expect(onDownloadError).toHaveBeenCalledWith('Local AI setup encountered an issue');
  });

  it('starts the recommended-preset bootstrap flow once', async () => {
    const { bootstrapLocalAiWithRecommendedPreset } =
      await import('../../../../utils/localAiBootstrap');

    const onNext = vi.fn();
    renderWithProviders(<LocalAIStep onNext={onNext} />);

    fireEvent.click(screen.getByRole('button', { name: /continue/i }));

    expect(bootstrapLocalAiWithRecommendedPreset).toHaveBeenCalledOnce();
    expect(bootstrapLocalAiWithRecommendedPreset).toHaveBeenCalledWith(false, '[LocalAIStep]');
  });

  it('double-click guard: download functions called only once', async () => {
    const { bootstrapLocalAiWithRecommendedPreset } =
      await import('../../../../utils/localAiBootstrap');
    vi.mocked(bootstrapLocalAiWithRecommendedPreset).mockResolvedValue({} as never);

    const onNext = vi.fn();
    renderWithProviders(<LocalAIStep onNext={onNext} />);

    const button = screen.getByRole('button', { name: /continue/i });
    fireEvent.click(button);
    fireEvent.click(button);

    expect(onNext).toHaveBeenCalledOnce();
    expect(bootstrapLocalAiWithRecommendedPreset).toHaveBeenCalledOnce();
  });
});
