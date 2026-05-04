import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  bootstrapLocalAiWithRecommendedPreset,
  ensureRecommendedLocalAiPresetIfNeeded,
} from '../localAiBootstrap';

vi.mock('../tauriCommands', () => ({
  openhumanLocalAiApplyPreset: vi.fn(),
  openhumanLocalAiDownloadAllAssets: vi.fn(),
  openhumanLocalAiPresets: vi.fn(),
}));

describe('localAiBootstrap', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('applies the recommended preset before starting background downloads when no tier is selected', async () => {
    const tauriCommands = await import('../tauriCommands');
    vi.mocked(tauriCommands.openhumanLocalAiPresets).mockResolvedValue({
      presets: [],
      recommended_tier: 'ram_2_4gb',
      current_tier: 'ram_2_4gb',
      selected_tier: null,
      device: {
        total_ram_bytes: 32 * 1024 * 1024 * 1024,
        cpu_count: 8,
        cpu_brand: 'Test CPU',
        os_name: 'macOS',
        os_version: '15',
        has_gpu: true,
        gpu_description: 'Test GPU',
      },
    });
    vi.mocked(tauriCommands.openhumanLocalAiApplyPreset).mockResolvedValue({
      applied_tier: 'ram_2_4gb',
      chat_model_id: 'gemma3:1b-it-qat',
      vision_model_id: '',
      embedding_model_id: 'all-minilm:latest',
      quantization: 'qat',
    });
    vi.mocked(tauriCommands.openhumanLocalAiDownloadAllAssets).mockResolvedValue({
      result: { state: 'downloading', progress: 0 } as never,
      logs: [],
    });

    const result = await bootstrapLocalAiWithRecommendedPreset(false, '[test]');

    expect(tauriCommands.openhumanLocalAiPresets).toHaveBeenCalledOnce();
    expect(tauriCommands.openhumanLocalAiApplyPreset).toHaveBeenCalledWith('ram_2_4gb');
    expect(tauriCommands.openhumanLocalAiDownloadAllAssets).toHaveBeenCalledWith(false);
    expect(
      vi.mocked(tauriCommands.openhumanLocalAiApplyPreset).mock.invocationCallOrder[0]
    ).toBeLessThan(
      vi.mocked(tauriCommands.openhumanLocalAiDownloadAllAssets).mock.invocationCallOrder[0]
    );
    expect(result.preset.hadSelectedTier).toBe(false);
    expect(result.preset.appliedTier).toBe('ram_2_4gb');
  });

  it('skips preset application when a tier is already selected', async () => {
    const tauriCommands = await import('../tauriCommands');
    vi.mocked(tauriCommands.openhumanLocalAiPresets).mockResolvedValue({
      presets: [],
      recommended_tier: 'ram_2_4gb',
      current_tier: 'ram_2_4gb',
      selected_tier: 'ram_2_4gb',
      device: {
        total_ram_bytes: 32 * 1024 * 1024 * 1024,
        cpu_count: 8,
        cpu_brand: 'Test CPU',
        os_name: 'macOS',
        os_version: '15',
        has_gpu: true,
        gpu_description: 'Test GPU',
      },
    });

    const result = await ensureRecommendedLocalAiPresetIfNeeded('[test]');

    expect(tauriCommands.openhumanLocalAiApplyPreset).not.toHaveBeenCalled();
    expect(result.hadSelectedTier).toBe(true);
    expect(result.selectedTier).toBe('ram_2_4gb');
  });
});
