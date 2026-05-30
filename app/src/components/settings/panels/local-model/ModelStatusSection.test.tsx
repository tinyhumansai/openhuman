import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { LocalAiDiagnostics } from '../../../../utils/tauriCommands';
import ModelStatusSection from './ModelStatusSection';

const defaultProps = {
  status: null,
  downloads: null,
  diagnostics: null,
  isDiagnosticsLoading: false,
  diagnosticsError: '',
  statusError: '',
  isTriggeringDownload: false,
  bootstrapMessage: '',
  progress: 0,
  isIndeterminateDownload: false,
  isInstalling: false,
  isInstallError: false,
  showErrorDetail: false,
  ollamaPathInput: '',
  isSettingPath: false,
  downloadedText: '',
  speedText: '',
  etaText: '',
  statusTone: (_state: string) => '',
  runtimeEnabled: true,
  ollamaBaseUrlInput: 'http://localhost:11434',
  isTestingConnection: false,
  connectionTestResult: null,
  isSavingUrl: false,
  savedOllamaBaseUrl: 'http://localhost:11434',
  onRefreshStatus: vi.fn(),
  onTriggerDownload: vi.fn(),
  onSetOllamaPath: vi.fn(),
  onClearOllamaPath: vi.fn(),
  onSetOllamaPathInput: vi.fn(),
  onToggleErrorDetail: vi.fn(),
  onRunDiagnostics: vi.fn(),
  onRepairAction: vi.fn(),
  onSetOllamaBaseUrlInput: vi.fn(),
  onTestConnection: vi.fn(),
  onSaveOllamaBaseUrl: vi.fn(),
  onResetOllamaBaseUrl: vi.fn(),
};

const makeDiagnostics = (overrides: Partial<LocalAiDiagnostics> = {}): LocalAiDiagnostics => ({
  ollama_running: true,
  ollama_base_url: 'http://localhost:11434',
  ollama_binary_path: '/usr/local/bin/ollama',
  installed_models: [],
  expected: {
    chat_model: 'gemma3:1b-it-qat',
    chat_found: true,
    embedding_model: 'nomic-embed-text',
    embedding_found: true,
    vision_model: 'llava',
    vision_found: false,
  },
  issues: [],
  repair_actions: [],
  ok: true,
  ...overrides,
});

describe('ModelStatusSection diagnostics', () => {
  it('still renders runtime status when runtime is disabled', () => {
    render(<ModelStatusSection {...defaultProps} runtimeEnabled={false} />);

    expect(screen.getByText('Runtime Status')).toBeTruthy();
    expect(screen.getByText('Refresh')).toBeTruthy();
  });

  it('shows the base URL being checked', () => {
    render(
      <ModelStatusSection
        {...defaultProps}
        diagnostics={makeDiagnostics({ ollama_base_url: 'http://192.168.1.5:11434' })}
      />
    );
    expect(screen.getByTitle('http://192.168.1.5:11434')).toBeTruthy();
  });

  it('shows Running when server is up', () => {
    render(
      <ModelStatusSection
        {...defaultProps}
        diagnostics={makeDiagnostics({ ollama_running: true })}
      />
    );
    expect(screen.getByText('Running')).toBeTruthy();
  });

  it('shows Not running when server is down', () => {
    render(
      <ModelStatusSection
        {...defaultProps}
        diagnostics={makeDiagnostics({ ollama_running: false })}
      />
    );
    expect(screen.getByText('Not running')).toBeTruthy();
  });

  it('shows Running via external process when binary is null but server is running', () => {
    render(
      <ModelStatusSection
        {...defaultProps}
        diagnostics={makeDiagnostics({ ollama_binary_path: null, ollama_running: true })}
      />
    );
    expect(screen.getByText('Running via external process')).toBeTruthy();
  });

  it('shows Not found when binary is null and server is not running', () => {
    render(
      <ModelStatusSection
        {...defaultProps}
        diagnostics={makeDiagnostics({ ollama_binary_path: null, ollama_running: false })}
      />
    );
    expect(screen.getByText('Not found')).toBeTruthy();
  });

  it('shows the binary path when set', () => {
    render(
      <ModelStatusSection
        {...defaultProps}
        diagnostics={makeDiagnostics({ ollama_binary_path: '/opt/homebrew/bin/ollama' })}
      />
    );
    expect(screen.getByText('/opt/homebrew/bin/ollama')).toBeTruthy();
  });

  it('renders manual-management guidance when diagnostics fail', () => {
    render(
      <ModelStatusSection
        {...defaultProps}
        diagnostics={makeDiagnostics({ ok: false, issues: ['Ollama server is not running'] })}
      />
    );
    expect(
      screen.getByText(/Manage the Ollama process and model pulls outside OpenHuman/)
    ).toBeTruthy();
  });

  it('does not render repair actions section when repair_actions is empty', () => {
    render(
      <ModelStatusSection {...defaultProps} diagnostics={makeDiagnostics({ repair_actions: [] })} />
    );
    expect(screen.queryByText('Suggested Fixes')).toBeNull();
  });

  it('shows all checks passed when ok is true', () => {
    render(<ModelStatusSection {...defaultProps} diagnostics={makeDiagnostics({ ok: true })} />);
    expect(screen.getByText('All checks passed')).toBeTruthy();
  });

  it('shows issue count when ok is false', () => {
    render(
      <ModelStatusSection
        {...defaultProps}
        diagnostics={makeDiagnostics({
          ok: false,
          issues: ['issue one', 'issue two'],
          repair_actions: [],
        })}
      />
    );
    expect(screen.getByText('2 issue(s) found')).toBeTruthy();
  });

  it('renders prompt text when diagnostics is null', () => {
    render(<ModelStatusSection {...defaultProps} diagnostics={null} />);
    expect(screen.getByText(/Click.*Run Diagnostics/)).toBeTruthy();
  });

  it('shows external-runtime guidance when ollama is unavailable', () => {
    render(
      <ModelStatusSection
        {...defaultProps}
        downloads={{
          state: 'idle',
          warning: null,
          progress: 0,
          downloaded_bytes: null,
          total_bytes: null,
          speed_bps: null,
          eta_seconds: null,
          ollama_available: false,
          chat: {
            id: 'gemma3:1b-it-qat',
            provider: 'ollama',
            state: 'missing',
            progress: null,
            downloaded_bytes: null,
            total_bytes: null,
            speed_bps: null,
            eta_seconds: null,
            warning: null,
            path: null,
          },
          vision: {
            id: '',
            provider: 'ollama',
            state: 'missing',
            progress: null,
            downloaded_bytes: null,
            total_bytes: null,
            speed_bps: null,
            eta_seconds: null,
            warning: null,
            path: null,
          },
          embedding: {
            id: 'bge-m3',
            provider: 'ollama',
            state: 'missing',
            progress: null,
            downloaded_bytes: null,
            total_bytes: null,
            speed_bps: null,
            eta_seconds: null,
            warning: null,
            path: null,
          },
          stt: {
            id: 'whisper',
            provider: 'whisper',
            state: 'missing',
            progress: null,
            downloaded_bytes: null,
            total_bytes: null,
            speed_bps: null,
            eta_seconds: null,
            warning: null,
            path: null,
          },
          tts: {
            id: 'piper',
            provider: 'piper',
            state: 'missing',
            progress: null,
            downloaded_bytes: null,
            total_bytes: null,
            speed_bps: null,
            eta_seconds: null,
            warning: null,
            path: null,
          },
        }}
      />
    );

    expect(screen.getByText('Ollama runtime unavailable')).toBeTruthy();
    expect(screen.getByText(/external inference runtime/)).toBeTruthy();
    expect(screen.getByText('Ollama docs')).toBeTruthy();
  });

  it('renders docs link instead of install controls when ollama is unavailable', () => {
    render(
      <ModelStatusSection
        {...defaultProps}
        downloads={{
          state: 'idle',
          warning: null,
          progress: 0,
          downloaded_bytes: null,
          total_bytes: null,
          speed_bps: null,
          eta_seconds: null,
          ollama_available: false,
          chat: {
            id: 'gemma3:1b-it-qat',
            provider: 'ollama',
            state: 'missing',
            progress: null,
            downloaded_bytes: null,
            total_bytes: null,
            speed_bps: null,
            eta_seconds: null,
            warning: null,
            path: null,
          },
          vision: {
            id: '',
            provider: 'ollama',
            state: 'missing',
            progress: null,
            downloaded_bytes: null,
            total_bytes: null,
            speed_bps: null,
            eta_seconds: null,
            warning: null,
            path: null,
          },
          embedding: {
            id: 'bge-m3',
            provider: 'ollama',
            state: 'missing',
            progress: null,
            downloaded_bytes: null,
            total_bytes: null,
            speed_bps: null,
            eta_seconds: null,
            warning: null,
            path: null,
          },
          stt: {
            id: 'whisper',
            provider: 'whisper',
            state: 'missing',
            progress: null,
            downloaded_bytes: null,
            total_bytes: null,
            speed_bps: null,
            eta_seconds: null,
            warning: null,
            path: null,
          },
          tts: {
            id: 'piper',
            provider: 'piper',
            state: 'missing',
            progress: null,
            downloaded_bytes: null,
            total_bytes: null,
            speed_bps: null,
            eta_seconds: null,
            warning: null,
            path: null,
          },
        }}
      />
    );

    expect(screen.queryByRole('button', { name: 'Install Ollama' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Set Path' })).toBeNull();
    expect(screen.getByRole('link', { name: 'Ollama docs' })).toBeTruthy();
  });

  it('accepts a model that meets the context minimum', () => {
    render(
      <ModelStatusSection
        {...defaultProps}
        diagnostics={makeDiagnostics({
          installed_models: [
            {
              name: 'bge-m3:latest',
              context_length: 8192,
              eligibility: { status: 'ok', context_length: 8192 },
            },
          ],
        })}
      />
    );
    expect(screen.getByText('8,192 ctx ✓')).toBeTruthy();
  });

  it('rejects and visually flags a model below the context minimum', () => {
    render(
      <ModelStatusSection
        {...defaultProps}
        diagnostics={makeDiagnostics({
          installed_models: [
            {
              name: 'tiny-embed:latest',
              context_length: 2048,
              eligibility: { status: 'below_minimum', context_length: 2048, required: 8192 },
            },
          ],
          issues: [
            'Embedding model `tiny-embed:latest` has a 2048-token context window; the memory layer requires at least 8192.',
          ],
          ok: false,
        })}
      />
    );
    expect(screen.getByText('2,048 ctx - below 8,192 min')).toBeTruthy();
    // Model name is rendered in the rejection (red) treatment.
    const name = screen.getByTitle('tiny-embed:latest');
    expect(name.className).toContain('text-red-700');
  });

  it('marks an unknown context window without rejecting it', () => {
    render(
      <ModelStatusSection
        {...defaultProps}
        diagnostics={makeDiagnostics({
          installed_models: [
            { name: 'mystery:latest', eligibility: { status: 'unknown', required: 8192 } },
          ],
        })}
      />
    );
    expect(screen.getByText('ctx unknown')).toBeTruthy();
  });

  it('renders models with no eligibility (older core) without a badge', () => {
    render(
      <ModelStatusSection
        {...defaultProps}
        diagnostics={makeDiagnostics({ installed_models: [{ name: 'legacy:latest', size: 1234 }] })}
      />
    );
    expect(screen.getByText('legacy:latest')).toBeTruthy();
    expect(screen.queryByText(/ctx/)).toBeNull();
  });
});

describe('ModelStatusSection — Ollama server URL', () => {
  it('renders the URL input with the default value', () => {
    render(<ModelStatusSection {...defaultProps} />);
    const input = screen.getByPlaceholderText('http://localhost:11434') as HTMLInputElement;
    expect(input).toBeTruthy();
    expect(input.value).toBe('http://localhost:11434');
  });

  it('shows a validation error for a bad URL', () => {
    render(
      <ModelStatusSection
        {...defaultProps}
        ollamaBaseUrlInput="not-a-url"
        savedOllamaBaseUrl="http://localhost:11434"
      />
    );
    expect(screen.getByText(/http:\/\/ or https:\/\//i)).toBeTruthy();
  });

  it('disables Save when URL is unchanged', () => {
    render(
      <ModelStatusSection
        {...defaultProps}
        ollamaBaseUrlInput="http://localhost:11434"
        savedOllamaBaseUrl="http://localhost:11434"
      />
    );
    const saveBtn = screen.getByRole('button', { name: 'Save' });
    expect((saveBtn as HTMLButtonElement).disabled).toBe(true);
  });

  it('enables Save when URL has changed and is valid', () => {
    render(
      <ModelStatusSection
        {...defaultProps}
        ollamaBaseUrlInput="http://192.168.1.5:11434"
        savedOllamaBaseUrl="http://localhost:11434"
      />
    );
    const saveBtn = screen.getByRole('button', { name: 'Save' });
    expect((saveBtn as HTMLButtonElement).disabled).toBe(false);
  });

  it('shows reachable status after a successful test', () => {
    render(
      <ModelStatusSection
        {...defaultProps}
        connectionTestResult={{ reachable: true, models_count: 3 }}
      />
    );
    expect(screen.getByText(/Reachable/)).toBeTruthy();
    expect(screen.getByText(/3 models/)).toBeTruthy();
  });

  it('shows unreachable status after a failed test', () => {
    render(
      <ModelStatusSection
        {...defaultProps}
        connectionTestResult={{ reachable: false, error: 'connection refused', models_count: null }}
      />
    );
    expect(screen.getByText(/Unreachable/)).toBeTruthy();
    expect(screen.getByText(/connection refused/)).toBeTruthy();
  });

  it('calls onTestConnection when Test Connection is clicked', async () => {
    const onTestConnection = vi.fn();
    render(
      <ModelStatusSection
        {...defaultProps}
        ollamaBaseUrlInput="http://localhost:11434"
        onTestConnection={onTestConnection}
      />
    );
    const testBtn = screen.getByRole('button', { name: /Test Connection/ });
    testBtn.click();
    expect(onTestConnection).toHaveBeenCalledTimes(1);
  });

  it('calls onResetOllamaBaseUrl when Reset to default is clicked', () => {
    const onResetOllamaBaseUrl = vi.fn();
    render(<ModelStatusSection {...defaultProps} onResetOllamaBaseUrl={onResetOllamaBaseUrl} />);
    const resetBtn = screen.getByRole('button', { name: /Reset to default/ });
    resetBtn.click();
    expect(onResetOllamaBaseUrl).toHaveBeenCalledTimes(1);
  });

  it('calls onSetOllamaBaseUrlInput when the URL input changes', () => {
    const onSetOllamaBaseUrlInput = vi.fn();
    render(
      <ModelStatusSection {...defaultProps} onSetOllamaBaseUrlInput={onSetOllamaBaseUrlInput} />
    );
    const input = screen.getByPlaceholderText('http://localhost:11434');
    fireEvent.change(input, { target: { value: 'http://192.168.1.5:11434' } });
    expect(onSetOllamaBaseUrlInput).toHaveBeenCalledWith('http://192.168.1.5:11434');
  });

  it('shows spinner when isTestingConnection is true', () => {
    render(<ModelStatusSection {...defaultProps} isTestingConnection={true} />);
    const testBtn = screen.getByRole('button', { name: /Test Connection/i });
    expect(testBtn.querySelector('.animate-spin')).toBeTruthy();
  });

  it('shows reachable result with model count when models_count is a number', () => {
    render(
      <ModelStatusSection
        {...defaultProps}
        connectionTestResult={{ reachable: true, models_count: 7 }}
      />
    );
    expect(screen.getByText(/Reachable/)).toBeTruthy();
    expect(screen.getByText(/7 models/)).toBeTruthy();
  });

  it('shows unreachable result with error text when reachable is false', () => {
    render(
      <ModelStatusSection
        {...defaultProps}
        connectionTestResult={{ reachable: false, error: 'dial tcp refused', models_count: null }}
      />
    );
    expect(screen.getByText(/Unreachable/)).toBeTruthy();
    expect(screen.getByText(/dial tcp refused/)).toBeTruthy();
  });

  it('shows validation error message for an invalid URL', () => {
    render(
      <ModelStatusSection
        {...defaultProps}
        ollamaBaseUrlInput="ftp://bad-scheme"
        savedOllamaBaseUrl="http://localhost:11434"
      />
    );
    expect(screen.getByText(/http:\/\/ or https:\/\//i)).toBeTruthy();
  });
});
