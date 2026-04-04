import { waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import Home from '../Home';

vi.mock('../../components/ConnectionIndicator', () => ({
  default: () => <div>Connection Indicator</div>,
}));

vi.mock('../../hooks/useUser', () => ({ useUser: () => ({ user: { firstName: 'Shrey' } }) }));

vi.mock('../../utils/localAiBootstrap', () => ({
  bootstrapLocalAiWithRecommendedPreset: vi.fn(),
  ensureRecommendedLocalAiPresetIfNeeded: vi.fn(),
  triggerLocalAiAssetBootstrap: vi.fn(),
}));

vi.mock('../../utils/tauriCommands', () => ({
  isTauri: vi.fn(() => true),
  openhumanLocalAiStatus: vi.fn(),
}));

describe('Home local AI bootstrap', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('auto-applies the recommended preset and starts bootstrap on first-run idle state', async () => {
    const bootstrapUtils = await import('../../utils/localAiBootstrap');
    const tauriCommands = await import('../../utils/tauriCommands');

    vi.mocked(tauriCommands.openhumanLocalAiStatus).mockResolvedValue({
      result: { state: 'idle', model_id: 'gemma3:4b-it-qat' } as never,
      logs: [],
    });
    vi.mocked(bootstrapUtils.ensureRecommendedLocalAiPresetIfNeeded).mockResolvedValue({
      presets: {} as never,
      recommendedTier: 'high',
      selectedTier: null,
      hadSelectedTier: false,
      appliedTier: 'high',
    });
    vi.mocked(bootstrapUtils.triggerLocalAiAssetBootstrap).mockResolvedValue({
      result: { state: 'downloading', progress: 0 } as never,
      logs: [],
    });

    renderWithProviders(<Home />);

    await waitFor(() => {
      expect(bootstrapUtils.ensureRecommendedLocalAiPresetIfNeeded).toHaveBeenCalledWith(
        '[Home first-run]'
      );
      expect(bootstrapUtils.triggerLocalAiAssetBootstrap).toHaveBeenCalledWith(
        false,
        '[Home first-run]'
      );
    });
  });

  it('does not auto-bootstrap when a tier is already selected', async () => {
    const bootstrapUtils = await import('../../utils/localAiBootstrap');
    const tauriCommands = await import('../../utils/tauriCommands');

    vi.mocked(tauriCommands.openhumanLocalAiStatus).mockResolvedValue({
      result: { state: 'idle', model_id: 'gemma3:12b-it-q4_K_M' } as never,
      logs: [],
    });
    vi.mocked(bootstrapUtils.ensureRecommendedLocalAiPresetIfNeeded).mockResolvedValue({
      presets: {} as never,
      recommendedTier: 'high',
      selectedTier: 'high',
      hadSelectedTier: true,
      appliedTier: null,
    });

    renderWithProviders(<Home />);

    await waitFor(() => {
      expect(bootstrapUtils.ensureRecommendedLocalAiPresetIfNeeded).toHaveBeenCalledWith(
        '[Home first-run]'
      );
    });
    expect(bootstrapUtils.triggerLocalAiAssetBootstrap).not.toHaveBeenCalled();
  });

  it('retries the first-run bootstrap trigger after preset application if the first trigger attempt fails', async () => {
    const bootstrapUtils = await import('../../utils/localAiBootstrap');
    const tauriCommands = await import('../../utils/tauriCommands');

    vi.mocked(tauriCommands.openhumanLocalAiStatus).mockResolvedValue({
      result: { state: 'idle', model_id: 'gemma3:4b-it-qat' } as never,
      logs: [],
    });
    vi.mocked(bootstrapUtils.ensureRecommendedLocalAiPresetIfNeeded)
      .mockResolvedValueOnce({
        presets: {} as never,
        recommendedTier: 'high',
        selectedTier: null,
        hadSelectedTier: false,
        appliedTier: 'high',
      })
      .mockResolvedValueOnce({
        presets: {} as never,
        recommendedTier: 'high',
        selectedTier: 'high',
        hadSelectedTier: true,
        appliedTier: null,
      });
    vi.mocked(bootstrapUtils.triggerLocalAiAssetBootstrap)
      .mockRejectedValueOnce(new Error('transient failure'))
      .mockResolvedValueOnce({ result: { state: 'downloading', progress: 0 } as never, logs: [] });

    renderWithProviders(<Home />);

    await waitFor(
      () => {
        expect(bootstrapUtils.triggerLocalAiAssetBootstrap).toHaveBeenCalledTimes(2);
      },
      { timeout: 3000 }
    );
    expect(bootstrapUtils.triggerLocalAiAssetBootstrap).toHaveBeenNthCalledWith(
      1,
      false,
      '[Home first-run]'
    );
    expect(bootstrapUtils.triggerLocalAiAssetBootstrap).toHaveBeenNthCalledWith(
      2,
      false,
      '[Home first-run]'
    );
  });
});
