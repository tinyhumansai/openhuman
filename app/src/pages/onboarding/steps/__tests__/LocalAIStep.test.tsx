import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import LocalAIStep from '../LocalAIStep';

vi.mock('../../../../utils/localAiBootstrap', () => ({
  bootstrapLocalAiWithRecommendedPreset: vi.fn().mockResolvedValue({} as never),
  ensureRecommendedLocalAiPresetIfNeeded: vi
    .fn()
    .mockResolvedValue({
      presets: { recommend_disabled: false },
      recommendedTier: 'ram_2_4gb',
      selectedTier: null,
      hadSelectedTier: false,
      appliedTier: null,
    } as never),
}));

describe('LocalAIStep', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('happy path: advances immediately and calls onNext with correct payload', async () => {
    const onNext = vi.fn();
    renderWithProviders(<LocalAIStep onNext={onNext} />);

    const button = await screen.findByRole('button', { name: /continue/i });
    fireEvent.click(button);

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

    const button = await screen.findByRole('button', { name: /continue/i });
    fireEvent.click(button);

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

    const button = await screen.findByRole('button', { name: /continue/i });
    fireEvent.click(button);

    expect(bootstrapLocalAiWithRecommendedPreset).toHaveBeenCalledOnce();
    expect(bootstrapLocalAiWithRecommendedPreset).toHaveBeenCalledWith(false, '[LocalAIStep]');
  });

  it('double-click guard: download functions called only once', async () => {
    const { bootstrapLocalAiWithRecommendedPreset } =
      await import('../../../../utils/localAiBootstrap');
    vi.mocked(bootstrapLocalAiWithRecommendedPreset).mockResolvedValue({} as never);

    const onNext = vi.fn();
    renderWithProviders(<LocalAIStep onNext={onNext} />);

    const button = await screen.findByRole('button', { name: /continue/i });
    fireEvent.click(button);
    fireEvent.click(button);

    expect(onNext).toHaveBeenCalledOnce();
    expect(bootstrapLocalAiWithRecommendedPreset).toHaveBeenCalledOnce();
  });

  it('shows cloud fallback UI when device is below RAM floor', async () => {
    const { ensureRecommendedLocalAiPresetIfNeeded } =
      await import('../../../../utils/localAiBootstrap');
    vi.mocked(ensureRecommendedLocalAiPresetIfNeeded).mockResolvedValue({
      presets: { recommend_disabled: true } as never,
      recommendedTier: 'ram_2_4gb',
      selectedTier: null,
      hadSelectedTier: false,
      appliedTier: null,
    });

    const onNext = vi.fn();
    renderWithProviders(<LocalAIStep onNext={onNext} />);

    const cloudButton = await screen.findByRole('button', { name: /continue with cloud/i });
    expect(cloudButton).toBeTruthy();

    fireEvent.click(cloudButton);
    expect(onNext).toHaveBeenCalledWith({ consentGiven: false, downloadStarted: false });
  });

  it('allows force-enabling local AI on low-RAM device', async () => {
    const { ensureRecommendedLocalAiPresetIfNeeded, bootstrapLocalAiWithRecommendedPreset } =
      await import('../../../../utils/localAiBootstrap');
    vi.mocked(ensureRecommendedLocalAiPresetIfNeeded).mockResolvedValue({
      presets: { recommend_disabled: true } as never,
      recommendedTier: 'ram_2_4gb',
      selectedTier: null,
      hadSelectedTier: false,
      appliedTier: null,
    });

    const onNext = vi.fn();
    renderWithProviders(<LocalAIStep onNext={onNext} />);

    const forceButton = await screen.findByRole('button', { name: /use local ai anyway/i });
    fireEvent.click(forceButton);

    expect(onNext).toHaveBeenCalledWith({ consentGiven: true, downloadStarted: true });
    expect(bootstrapLocalAiWithRecommendedPreset).toHaveBeenCalledOnce();
  });
});
