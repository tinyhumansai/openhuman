import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import DeviceCapabilitySection from './DeviceCapabilitySection';

const mockApplyPreset = vi.fn();

vi.mock('../../../../utils/tauriCommands', () => ({
  openhumanLocalAiApplyPreset: (...args: unknown[]) => mockApplyPreset(...args),
}));

const makePresetsData = (overrides: Record<string, unknown> = {}) => ({
  presets: [
    {
      tier: 'ram_2_4gb',
      label: '2-4 GB',
      description: 'Small local tier',
      chat_model_id: 'gemma3:1b-it-qat',
      vision_model_id: '',
      embedding_model_id: 'bge-m3',
      quantization: 'q4',
      vision_mode: 'disabled',
      supports_screen_summary: false,
      target_ram_gb: 4,
      min_ram_gb: 2,
      approx_download_gb: 1.2,
    },
  ],
  recommended_tier: 'ram_2_4gb',
  current_tier: 'ram_2_4gb',
  selected_tier: 'ram_2_4gb',
  recommend_disabled: false,
  local_ai_enabled: true,
  device: {
    total_ram_bytes: 16 * 1024 * 1024 * 1024,
    cpu_count: 8,
    cpu_brand: 'Test CPU',
    os_name: 'macOS',
    os_version: '15',
    has_gpu: true,
    gpu_description: 'Test GPU',
  },
  ...overrides,
});

describe('DeviceCapabilitySection', () => {
  beforeEach(() => {
    mockApplyPreset.mockReset();
  });

  it('renders external runtime guidance when ollama is unavailable', () => {
    render(
      <DeviceCapabilitySection
        presetsData={makePresetsData()}
        presetsLoading={false}
        presetError=""
        presetSuccess={null}
        formatRamGb={() => '16 GB'}
        ollamaAvailable={false}
      />
    );

    expect(screen.getByText(/Run Ollama first/i)).toBeTruthy();
    expect(screen.getByRole('link', { name: 'Ollama docs' })).toBeTruthy();
    expect(screen.getByTitle('Run Ollama first to use this tier')).toBeTruthy();
  });

  it('allows selecting the disabled cloud fallback tier', async () => {
    mockApplyPreset.mockResolvedValueOnce({ applied_tier: 'disabled' });

    render(
      <DeviceCapabilitySection
        presetsData={makePresetsData({ local_ai_enabled: false })}
        presetsLoading={false}
        presetError=""
        presetSuccess={null}
        formatRamGb={() => '16 GB'}
      />
    );

    fireEvent.click(screen.getByRole('button', { name: /Disabled.*0 GB/i }));

    await waitFor(() => {
      expect(mockApplyPreset).toHaveBeenCalledWith('disabled');
    });
  });
});
