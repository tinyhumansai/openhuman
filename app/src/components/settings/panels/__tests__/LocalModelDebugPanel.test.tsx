import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import LocalModelDebugPanel from '../LocalModelDebugPanel';

const { mockNavigateBack } = vi.hoisted(() => ({ mockNavigateBack: vi.fn() }));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: mockNavigateBack, breadcrumbs: [] }),
}));

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

describe('LocalModelDebugPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockLocalAiStatus.mockResolvedValue({ result: null });
    mockLocalAiAssetsStatus.mockResolvedValue({ result: null });
    mockLocalAiDownloadsProgress.mockResolvedValue({ result: null });
    mockGetConfig.mockResolvedValue({ result: { config: {} } });
    mockLocalAiDiagnostics.mockResolvedValue({
      ok: true,
      ollama_running: false,
      ollama_base_url: null,
      ollama_binary_path: null,
      installed_models: [],
      expected: {
        chat_model: '',
        chat_found: false,
        embedding_model: '',
        embedding_found: false,
        vision_model: '',
        vision_found: false,
      },
      issues: [],
      repair_actions: [],
    });
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
});
