import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { LocalAiDiagnostics, RepairAction } from '../../../../utils/tauriCommands';
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
  onRefreshStatus: vi.fn(),
  onTriggerDownload: vi.fn(),
  onSetOllamaPath: vi.fn(),
  onClearOllamaPath: vi.fn(),
  onSetOllamaPathInput: vi.fn(),
  onToggleErrorDetail: vi.fn(),
  onRunDiagnostics: vi.fn(),
  onRepairAction: vi.fn(),
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

  it('renders repair action buttons', () => {
    const repairActions: RepairAction[] = [
      { action: 'install_ollama' },
      { action: 'start_server', binary_path: '/usr/local/bin/ollama' },
      { action: 'pull_model', model: 'gemma3:1b-it-qat' },
    ];
    render(
      <ModelStatusSection
        {...defaultProps}
        diagnostics={makeDiagnostics({
          ok: false,
          issues: ['Ollama server is not running'],
          repair_actions: repairActions,
        })}
      />
    );
    expect(screen.getByText('Install Ollama')).toBeTruthy();
    expect(screen.getByText('Start Server')).toBeTruthy();
    expect(screen.getByText('Pull gemma3:1b-it-qat')).toBeTruthy();
  });

  it('calls onRepairAction with the correct action when button is clicked', () => {
    const onRepairAction = vi.fn();
    const repairActions: RepairAction[] = [{ action: 'install_ollama' }];
    render(
      <ModelStatusSection
        {...defaultProps}
        onRepairAction={onRepairAction}
        diagnostics={makeDiagnostics({
          ok: false,
          issues: ['Ollama server is not running'],
          repair_actions: repairActions,
        })}
      />
    );
    fireEvent.click(screen.getByText('Install Ollama'));
    expect(onRepairAction).toHaveBeenCalledWith({ action: 'install_ollama' });
  });

  it('calls onRepairAction with pull_model action', () => {
    const onRepairAction = vi.fn();
    const repairActions: RepairAction[] = [{ action: 'pull_model', model: 'gemma3:1b-it-qat' }];
    render(
      <ModelStatusSection
        {...defaultProps}
        onRepairAction={onRepairAction}
        diagnostics={makeDiagnostics({
          ok: false,
          issues: ['Chat model is not installed'],
          repair_actions: repairActions,
        })}
      />
    );
    fireEvent.click(screen.getByText('Pull gemma3:1b-it-qat'));
    expect(onRepairAction).toHaveBeenCalledWith({
      action: 'pull_model',
      model: 'gemma3:1b-it-qat',
    });
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
});
