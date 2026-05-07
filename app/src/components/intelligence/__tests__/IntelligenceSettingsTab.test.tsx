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
    // Accept both legacy (bare string) and the new request-object shape so
    // tests can assert on either call form.
    memoryTreeSetLlm.mockImplementation(
      async (req: 'cloud' | 'local' | { backend: 'cloud' | 'local' }) => {
        backend = typeof req === 'string' ? req : req.backend;
        return { current: backend };
      }
    );
  });

  // Helper: bootstrap into Local mode so the model assignment + catalog
  // render. Cloud is the default; clicking the Advanced radio flips to
  // local and renders the Ollama-related sections.
  async function flipToLocal() {
    await waitFor(() => {
      expect(screen.getByText('AI backend')).toBeInTheDocument();
    });
    const radios = screen.getAllByRole('radio');
    const localCard = radios.find(el => /Advanced/.test(el.textContent ?? ''));
    expect(localCard).toBeDefined();
    fireEvent.click(localCard!);
    await waitFor(() => {
      expect(screen.getByText('Model assignment')).toBeInTheDocument();
    });
  }

  it('renders the AI backend section in cloud mode (no local sections)', async () => {
    renderWithProviders(<IntelligenceSettingsTab />);

    await waitFor(() => {
      expect(screen.getByText('AI backend')).toBeInTheDocument();
    });
    // Cloud is default — local-only sections are hidden so cloud users
    // never see Ollama-related UI.
    expect(screen.queryByText('Model assignment')).not.toBeInTheDocument();
    expect(screen.queryByText('Model catalog')).not.toBeInTheDocument();
    // Currently-loaded panel was removed entirely (was dev-debug noise).
    expect(screen.queryByText('Currently loaded')).not.toBeInTheDocument();
  });

  it('hides Model assignment in Cloud mode and reveals it in Local mode', async () => {
    renderWithProviders(<IntelligenceSettingsTab />);
    await flipToLocal();

    // The new UI consolidates Extract + Summariser LLM into a single
    // Memory LLM picker (the underlying RPC still fans out to both
    // extract_model and summariser_model in config.toml).
    expect(screen.getByText('Memory LLM')).toBeInTheDocument();
    expect(screen.getByText('Embedder')).toBeInTheDocument();
    // Old separate dropdowns must be absent.
    expect(screen.queryByText('Extract LLM')).not.toBeInTheDocument();
    expect(screen.queryByText('Summariser LLM')).not.toBeInTheDocument();
  });

  it('shows model catalog rows with sizes (in local mode)', async () => {
    renderWithProviders(<IntelligenceSettingsTab />);
    await flipToLocal();

    await waitFor(() => {
      expect(screen.getAllByText('qwen2.5:0.5b').length).toBeGreaterThanOrEqual(1);
    });
    // Each model can appear in the Memory LLM dropdown AND the catalog,
    // so use getAllByText. Just confirm the catalog has at least one of
    // each curated entry rendered somewhere on the screen.
    expect(screen.getAllByText('gemma3:1b-it-qat').length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText('gemma3:4b').length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText('gemma3:12b-it-qat').length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText('bge-m3').length).toBeGreaterThanOrEqual(1);

    // 3.3 GB is unique to gemma3:4b in the catalog row meta.
    expect(screen.getByText('3.3 GB')).toBeInTheDocument();
  });

  it('renders a Download action for models that are not installed', async () => {
    renderWithProviders(<IntelligenceSettingsTab />);
    await flipToLocal();

    // qwen2.5:0.5b is NOT in the diagnostics installed list, so it shows
    // a Download button.
    await waitFor(() => {
      expect(screen.getByText('qwen2.5:0.5b')).toBeInTheDocument();
    });

    const downloadButtons = screen.getAllByRole('button', { name: 'Download' });
    expect(downloadButtons.length).toBeGreaterThanOrEqual(1);
  });

  it('reads the backend via memoryTreeGetLlm on mount and persists toggles via memoryTreeSetLlm', async () => {
    renderWithProviders(<IntelligenceSettingsTab />);

    // Bootstrap: getMemoryTreeLlm must run once on mount.
    await waitFor(() => {
      expect(memoryTreeGetLlm).toHaveBeenCalled();
    });

    // Click Local — setMemoryTreeLlm must be called with the request
    // object form `{ backend: 'local' }`. settingsApi.ts always normalizes
    // to the request-object shape because the wrapper now accepts both
    // forms but the API layer translates camelCase options through the
    // object shape. Model fields are absent so the corresponding
    // config keys stay untouched.
    const radios = screen.getAllByRole('radio');
    const localCard = radios.find(el => /Advanced/.test(el.textContent ?? ''));
    fireEvent.click(localCard!);

    await waitFor(() => {
      expect(memoryTreeSetLlm).toHaveBeenCalledWith({ backend: 'local' });
    });

    // The mocked setter persists state in the closure, so the bootstrap
    // value of any subsequent get_llm call would now be 'local' — sanity
    // check that the closure flipped.
    const after = await memoryTreeGetLlm();
    expect(after.current).toBe('local');
  });

  it('persists Memory LLM dropdown changes via memoryTreeSetLlm with both extract_model and summariser_model', async () => {
    // The single Memory LLM picker fans out to BOTH extract_model and
    // summariser_model in one atomic write — the underlying schema keeps
    // the two keys separate so power users can split via the RPC, but the
    // UI consolidates them into one cognitive unit.
    renderWithProviders(<IntelligenceSettingsTab />);
    await flipToLocal();

    // Reset call history so the assertion below is scoped to the
    // dropdown change, not the earlier backend toggle.
    memoryTreeSetLlm.mockClear();

    // Pick a different memory LLM. `gemma3:12b-it-qat` is in the curated
    // catalog with both `extract` and `summariser` roles.
    const memorySelect = screen.getByLabelText(
      'Memory LLM (extract + summarise)'
    ) as HTMLSelectElement;
    fireEvent.change(memorySelect, { target: { value: 'gemma3:12b-it-qat' } });

    await waitFor(() => {
      expect(memoryTreeSetLlm).toHaveBeenCalledWith({
        backend: 'local',
        extract_model: 'gemma3:12b-it-qat',
        summariser_model: 'gemma3:12b-it-qat',
      });
    });
  });
});
