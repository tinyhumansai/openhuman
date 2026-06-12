import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import LocalModelDebugPanel from '../LocalModelDebugPanel';

const { mockNavigateBack } = vi.hoisted(() => ({ mockNavigateBack: vi.fn() }));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: mockNavigateBack, breadcrumbs: [] }),
}));

// No i18n mock: the context default resolves real English translations via resolveEn(),
// which is needed by existing tests that assert on actual English text (e.g. 'Test Connection',
// 'Reachable', 'http://localhost:11434' placeholder). New tests must also use real strings.

vi.mock('../components/SettingsHeader', () => ({ default: () => null }));

const mockGetConfig = vi.fn();
vi.mock('../../../../utils/tauriCommands/config', () => ({
  openhumanGetConfig: (...args: unknown[]) => mockGetConfig(...args),
}));

const mockLocalAiStatus = vi.fn();
const mockLocalAiAssetsStatus = vi.fn();
const mockLocalAiDownloadsProgress = vi.fn();
const mockLocalAiTestConnection = vi.fn();
const mockUpdateLocalAiSettings = vi.fn();
const mockLocalAiDiagnostics = vi.fn();

vi.mock('../../../../utils/tauriCommands', () => ({
  openhumanLocalAiStatus: (...args: unknown[]) => mockLocalAiStatus(...args),
  openhumanLocalAiAssetsStatus: (...args: unknown[]) => mockLocalAiAssetsStatus(...args),
  openhumanLocalAiDownloadsProgress: (...args: unknown[]) => mockLocalAiDownloadsProgress(...args),
  openhumanLocalAiTestConnection: (...args: unknown[]) => mockLocalAiTestConnection(...args),
  openhumanUpdateLocalAiSettings: (...args: unknown[]) => mockUpdateLocalAiSettings(...args),
  openhumanLocalAiDiagnostics: (...args: unknown[]) => mockLocalAiDiagnostics(...args),
  openhumanLocalAiSummarize: vi.fn().mockResolvedValue({ result: '' }),
  openhumanLocalAiPrompt: vi.fn().mockResolvedValue({ result: '' }),
  openhumanLocalAiEmbed: vi.fn().mockResolvedValue({ result: [] }),
  openhumanLocalAiVisionPrompt: vi.fn().mockResolvedValue({ result: '' }),
  openhumanLocalAiTranscribe: vi.fn().mockResolvedValue({ result: '' }),
  openhumanLocalAiTts: vi.fn().mockResolvedValue({ result: '' }),
  openhumanLocalAiDownloadAsset: vi.fn().mockResolvedValue({ result: null }),
}));

function renderPanel() {
  return render(
    <MemoryRouter>
      <LocalModelDebugPanel />
    </MemoryRouter>
  );
}

const makeDiagnostics = (overrides: Record<string, unknown> = {}) => ({
  ok: true,
  ollama_running: false,
  ollama_base_url: null,
  ollama_binary_path: null,
  installed_models: [],
  expected: {
    chat_model: 'llama3',
    chat_found: false,
    embedding_model: 'nomic-embed-text',
    embedding_found: false,
    vision_model: 'llava',
    vision_found: false,
  },
  issues: [],
  repair_actions: [],
  ...overrides,
});

describe('LocalModelDebugPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockLocalAiStatus.mockResolvedValue({ result: null });
    mockLocalAiAssetsStatus.mockResolvedValue({ result: null });
    mockLocalAiDownloadsProgress.mockResolvedValue({ result: null });
    mockGetConfig.mockResolvedValue({ result: { config: {} } });
    mockLocalAiDiagnostics.mockResolvedValue(makeDiagnostics());
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('renders the Ollama Server URL section with default URL', () => {
    renderPanel();
    const input = screen.getByPlaceholderText('http://localhost:11434') as HTMLInputElement;
    expect(input.value).toBe('http://localhost:11434');
  });

  it('seeds the URL input from config on mount', async () => {
    mockGetConfig.mockResolvedValue({
      result: { config: { local_ai: { base_url: 'http://192.168.1.5:11434' } } },
    });
    renderPanel();
    await waitFor(() => {
      const input = screen.getByPlaceholderText('http://localhost:11434') as HTMLInputElement;
      expect(input.value).toBe('http://192.168.1.5:11434');
    });
  });

  it('keeps the default URL when config returns no base_url', async () => {
    mockGetConfig.mockResolvedValue({ result: { config: { local_ai: {} } } });
    renderPanel();
    await waitFor(() => {
      const input = screen.getByPlaceholderText('http://localhost:11434') as HTMLInputElement;
      expect(input.value).toBe('http://localhost:11434');
    });
    expect(mockGetConfig).toHaveBeenCalledTimes(1);
  });

  it('calls openhumanLocalAiTestConnection when Test Connection is clicked', async () => {
    mockLocalAiTestConnection.mockResolvedValue({ reachable: true, models_count: 2 });
    renderPanel();
    const testBtn = screen.getByRole('button', { name: /Test Connection/i });
    fireEvent.click(testBtn);
    await waitFor(() => {
      expect(mockLocalAiTestConnection).toHaveBeenCalledWith('http://localhost:11434');
    });
  });

  it('shows reachable result after a successful connection test', async () => {
    mockLocalAiTestConnection.mockResolvedValue({ reachable: true, models_count: 5 });
    renderPanel();
    fireEvent.click(screen.getByRole('button', { name: /Test Connection/i }));
    await waitFor(() => expect(screen.getByText(/Reachable/)).toBeTruthy());
    expect(screen.getByText(/5 models/)).toBeTruthy();
  });

  it('shows unreachable result when connection test throws', async () => {
    mockLocalAiTestConnection.mockRejectedValue(new Error('connect ECONNREFUSED'));
    renderPanel();
    fireEvent.click(screen.getByRole('button', { name: /Test Connection/i }));
    await waitFor(() => expect(screen.getByText(/connect ECONNREFUSED/)).toBeTruthy());
  });

  it('saves the URL when Save is clicked after changing the input', async () => {
    mockUpdateLocalAiSettings.mockResolvedValue({ result: true });
    renderPanel();
    const urlInput = screen.getByPlaceholderText('http://localhost:11434');
    fireEvent.change(urlInput, { target: { value: 'http://192.168.1.5:11434' } });
    const saveBtn = await screen.findByRole('button', { name: 'Save' });
    expect((saveBtn as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(saveBtn);
    await waitFor(() => {
      expect(mockUpdateLocalAiSettings).toHaveBeenCalledWith({
        base_url: 'http://192.168.1.5:11434',
      });
    });
  });

  it('resets the URL to default when Reset to default is clicked', async () => {
    mockUpdateLocalAiSettings.mockResolvedValue({ result: true });
    renderPanel();
    const resetBtn = screen.getByRole('button', { name: /Reset to default/i });
    fireEvent.click(resetBtn);
    await waitFor(() => {
      expect(mockUpdateLocalAiSettings).toHaveBeenCalledWith({ base_url: null });
    });
    const urlInput = screen.getByPlaceholderText('http://localhost:11434') as HTMLInputElement;
    expect(urlInput.value).toBe('http://localhost:11434');
  });

  // ── statusTone function (line 53) — exercised via ModelStatusSection rendering

  it('renders ready state tone (line 53 — statusTone "ready")', async () => {
    mockLocalAiStatus.mockResolvedValue({
      result: {
        state: 'ready',
        provider: 'ollama',
        model_id: 'llama3',
        active_backend: 'cpu',
        last_latency_ms: 120,
        gen_toks_per_sec: 15.5,
        download_progress: null,
        downloaded_bytes: null,
        total_bytes: null,
        download_speed_bps: null,
        eta_seconds: null,
        error_category: null,
        error_detail: null,
        warning: null,
        backend_reason: null,
        model_path: null,
      },
    });
    renderPanel();
    // 'Runtime Status' is the English translation of 'settings.localModel.status.runtimeStatus'
    await waitFor(() => expect(screen.getByText('Runtime Status')).toBeInTheDocument());
    // statusTone('ready') → 'text-green-600 dark:text-green-300' (just confirm no crash)
  });

  it('renders degraded state (line 53 — statusTone "degraded")', async () => {
    mockLocalAiStatus.mockResolvedValue({
      result: {
        state: 'degraded',
        provider: null,
        model_id: null,
        active_backend: 'cpu',
        last_latency_ms: null,
        gen_toks_per_sec: null,
        download_progress: null,
        downloaded_bytes: null,
        total_bytes: null,
        download_speed_bps: null,
        eta_seconds: null,
        error_category: 'install',
        error_detail: 'checksum mismatch',
        warning: null,
        backend_reason: null,
        model_path: null,
      },
    });
    renderPanel();
    await waitFor(() => expect(screen.getByText('Runtime Status')).toBeInTheDocument());
  });

  it('renders disabled state (line 53 — statusTone "disabled")', async () => {
    mockLocalAiStatus.mockResolvedValue({
      result: {
        state: 'disabled',
        provider: null,
        model_id: null,
        active_backend: null,
        last_latency_ms: null,
        gen_toks_per_sec: null,
        download_progress: null,
        downloaded_bytes: null,
        total_bytes: null,
        download_speed_bps: null,
        eta_seconds: null,
        error_category: null,
        error_detail: null,
        warning: null,
        backend_reason: null,
        model_path: null,
      },
    });
    renderPanel();
    await waitFor(() => expect(screen.getByText('Runtime Status')).toBeInTheDocument());
  });

  // ── ModelStatusSection: statusError display (line 405) ────────────────────

  it('runs diagnostics via Run Diagnostics button', async () => {
    const diagnostics = makeDiagnostics({
      ok: true,
      ollama_running: true,
      ollama_base_url: 'http://localhost:11434',
      ollama_binary_path: '/usr/local/bin/ollama',
      installed_models: [
        {
          name: 'llama3:8b',
          size: 4815162342,
          eligibility: { status: 'ok', context_length: 8192, required: 2048 },
        },
      ],
    });
    mockLocalAiDiagnostics.mockResolvedValue(diagnostics);
    renderPanel();

    // 'Run Diagnostics' is the English translation of 'settings.localModel.status.runDiagnostics'
    fireEvent.click(screen.getByText('Run Diagnostics'));

    await waitFor(() => expect(mockLocalAiDiagnostics).toHaveBeenCalled());
    // After diagnostics run: 'All checks passed' (translation of 'settings.localModel.status.allChecksPassed')
    await waitFor(() => expect(screen.getByText('All checks passed')).toBeInTheDocument());
  });

  it('renders installed models with eligibility badge (line 446 — diagnostics flow)', async () => {
    mockLocalAiDiagnostics.mockResolvedValue(
      makeDiagnostics({
        ok: false,
        ollama_running: true,
        installed_models: [
          {
            name: 'tiny-model',
            size: 100000,
            eligibility: { status: 'below_minimum', context_length: 512, required: 2048 },
          },
        ],
        issues: ['Chat model not installed'],
      })
    );
    renderPanel();

    fireEvent.click(screen.getByText('Run Diagnostics'));

    await waitFor(() => expect(screen.getByText('tiny-model')).toBeInTheDocument());
    expect(screen.getByText(/512/)).toBeInTheDocument();
  });

  it('shows issues found when diagnostics has issues (line 446)', async () => {
    mockLocalAiDiagnostics.mockResolvedValue(
      makeDiagnostics({ ok: false, issues: ['Chat model not found', 'Embedding model not found'] })
    );
    renderPanel();

    fireEvent.click(screen.getByText('Run Diagnostics'));

    await waitFor(() => expect(screen.getByText('Chat model not found')).toBeInTheDocument());
    expect(screen.getByText('Embedding model not found')).toBeInTheDocument();
  });
});
