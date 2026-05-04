import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, type Mock, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import IntelligenceSettingsTab from '../IntelligenceSettingsTab';

// The orchestrator hits these RPCs on mount; the global tauriCommands mock
// in setup.ts only stubs auth/service helpers, so we extend it here with
// the local-AI surface the Settings tab uses, plus the new memory_tree
// LLM-selector RPCs that replaced the dev-time mock backend.
vi.mock('../../../utils/tauriCommands', () => ({
  isTauri: vi.fn(() => true),
  // memory_tree LLM selector — the BackendChooser polls these on mount and
  // again on every backend toggle. We track the value in a closure so the
  // set→get round-trip behaves like the real persistent core.
  memoryTreeGetLlm: vi.fn(),
  memoryTreeSetLlm: vi.fn(),
  openhumanLocalAiAssetsStatus: vi
    .fn()
    .mockResolvedValue({
      result: {
        chat: { state: 'NotInstalled', id: '', provider: 'ollama' },
        vision: { state: 'NotInstalled', id: '', provider: 'ollama' },
        embedding: { state: 'NotInstalled', id: '', provider: 'ollama' },
        stt: { state: 'NotInstalled', id: '', provider: 'ollama' },
        tts: { state: 'NotInstalled', id: '', provider: 'ollama' },
        quantization: 'q4_k_m',
      },
    }),
  openhumanLocalAiDiagnostics: vi.fn().mockResolvedValue({
    ollama_running: true,
    ollama_binary_path: '/usr/local/bin/ollama',
    installed_models: [
      { name: 'gemma3:1b-it-qat', size: 1_700_000_000, modified_at: null },
      { name: 'bge-m3', size: 1_300_000_000, modified_at: null },
    ],
    expected: {
      chat_model: 'gemma3:1b-it-qat',
      chat_found: true,
      embedding_model: 'bge-m3',
      embedding_found: true,
      vision_model: '',
      vision_found: false,
    },
    issues: [],
    ok: true,
  }),
  openhumanLocalAiStatus: vi
    .fn()
    .mockResolvedValue({
      result: {
        state: 'Ready',
        model_id: 'gemma3:1b-it-qat',
        chat_model_id: 'gemma3:1b-it-qat',
        vision_model_id: '',
        embedding_model_id: 'bge-m3',
        stt_model_id: '',
        tts_voice_id: '',
        quantization: 'q4_k_m',
        vision_state: 'idle',
        vision_mode: 'off',
        embedding_state: 'Ready',
        stt_state: 'idle',
        tts_state: 'idle',
        provider: 'ollama',
        active_backend: 'cpu',
        last_latency_ms: 142,
      },
    }),
  openhumanLocalAiPresets: vi
    .fn()
    .mockResolvedValue({
      presets: [],
      recommended_tier: 'minimal',
      current_tier: 'minimal',
      device: {
        total_ram_bytes: 16_000_000_000,
        cpu_count: 8,
        cpu_brand: 'Test CPU',
        os_name: 'macos',
        os_version: '14',
        has_gpu: false,
        gpu_description: null,
      },
      local_ai_enabled: false,
    }),
  openhumanLocalAiDownloadAsset: vi
    .fn()
    .mockResolvedValue({
      result: {
        chat: { state: 'Ready', id: 'gemma3:1b-it-qat', provider: 'ollama' },
        vision: { state: 'NotInstalled', id: '', provider: 'ollama' },
        embedding: { state: 'Ready', id: 'bge-m3', provider: 'ollama' },
        stt: { state: 'NotInstalled', id: '', provider: 'ollama' },
        tts: { state: 'NotInstalled', id: '', provider: 'ollama' },
        quantization: 'q4_k_m',
      },
    }),
}));

// Pull mocked references after vi.mock() has hoisted. Cast through unknown
// because the import here is the typed wrapper module shape.
const { memoryTreeGetLlm, memoryTreeSetLlm } =
  (await import('../../../utils/tauriCommands')) as unknown as {
    memoryTreeGetLlm: Mock;
    memoryTreeSetLlm: Mock;
  };

describe('IntelligenceSettingsTab', () => {
  beforeEach(() => {
    let backend: 'cloud' | 'local' = 'cloud';
    memoryTreeGetLlm.mockReset();
    memoryTreeSetLlm.mockReset();
    memoryTreeGetLlm.mockImplementation(async () => ({ current: backend }));
    memoryTreeSetLlm.mockImplementation(async (next: 'cloud' | 'local') => {
      backend = next;
      return { current: backend };
    });
  });

  it('renders the four section headings', async () => {
    renderWithProviders(<IntelligenceSettingsTab />);

    await waitFor(() => {
      expect(screen.getByText('AI backend')).toBeInTheDocument();
    });
    expect(screen.getByText('Model catalog')).toBeInTheDocument();
    expect(screen.getByText('Currently loaded')).toBeInTheDocument();
  });

  it('hides Model assignment in Cloud mode and reveals it in Local mode', async () => {
    renderWithProviders(<IntelligenceSettingsTab />);

    await waitFor(() => {
      expect(screen.getByText('AI backend')).toBeInTheDocument();
    });

    // Cloud is the default — Model assignment heading should be absent.
    expect(screen.queryByText('Model assignment')).not.toBeInTheDocument();

    // Click the Local card. Two radio buttons exist (Cloud and Local);
    // we pick the one tagged Advanced (the Cloud card is tagged Recommended).
    const radios = screen.getAllByRole('radio');
    const localCard = radios.find(el => /Advanced/.test(el.textContent ?? ''));
    expect(localCard).toBeDefined();
    fireEvent.click(localCard!);

    await waitFor(() => {
      expect(screen.getByText('Model assignment')).toBeInTheDocument();
    });

    // The role rows are rendered with their labels.
    expect(screen.getByText('Extract LLM')).toBeInTheDocument();
    expect(screen.getByText('Summariser LLM')).toBeInTheDocument();
    expect(screen.getByText('Embedder')).toBeInTheDocument();
  });

  it('shows model catalog rows with sizes', async () => {
    renderWithProviders(<IntelligenceSettingsTab />);

    await waitFor(() => {
      expect(screen.getByText('qwen2.5:0.5b')).toBeInTheDocument();
    });
    expect(screen.getByText('gemma3:1b-it-qat')).toBeInTheDocument();
    expect(screen.getByText('llama3.1:8b')).toBeInTheDocument();
    // bge-m3 appears in both the embedder lookup line and the catalog.
    expect(screen.getAllByText('bge-m3').length).toBeGreaterThanOrEqual(1);

    // 4.9 GB is unique to llama3.1:8b in the catalog.
    expect(screen.getByText('4.9 GB')).toBeInTheDocument();
  });

  it('renders a Download action for models that are not installed', async () => {
    renderWithProviders(<IntelligenceSettingsTab />);

    // qwen2.5:0.5b is NOT in the diagnostics installed list, so it shows
    // a Download button.
    await waitFor(() => {
      expect(screen.getByText('qwen2.5:0.5b')).toBeInTheDocument();
    });

    const downloadButtons = screen.getAllByRole('button', { name: 'Download' });
    expect(downloadButtons.length).toBeGreaterThanOrEqual(1);
  });

  it('renders the Currently loaded readout with the embedder row', async () => {
    renderWithProviders(<IntelligenceSettingsTab />);

    await waitFor(() => {
      expect(screen.getByText('Currently loaded')).toBeInTheDocument();
    });

    // The embedder row uses font-mono for the model id; we just check
    // it's present somewhere under the readout.
    const embedderHits = screen.getAllByText('bge-m3');
    expect(embedderHits.length).toBeGreaterThanOrEqual(1);
  });

  it('reads the backend via memoryTreeGetLlm on mount and persists toggles via memoryTreeSetLlm', async () => {
    renderWithProviders(<IntelligenceSettingsTab />);

    // Bootstrap: getMemoryTreeLlm must run once on mount.
    await waitFor(() => {
      expect(memoryTreeGetLlm).toHaveBeenCalled();
    });

    // Click Local — setMemoryTreeLlm must be called with 'local'.
    const radios = screen.getAllByRole('radio');
    const localCard = radios.find(el => /Advanced/.test(el.textContent ?? ''));
    fireEvent.click(localCard!);

    await waitFor(() => {
      expect(memoryTreeSetLlm).toHaveBeenCalledWith('local');
    });

    // The mocked setter persists state in the closure, so the bootstrap
    // value of any subsequent get_llm call would now be 'local' — sanity
    // check that the closure flipped.
    const after = await memoryTreeGetLlm();
    expect(after.current).toBe('local');
  });
});
