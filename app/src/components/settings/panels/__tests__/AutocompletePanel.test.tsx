import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import {
  type AutocompleteConfig,
  type AutocompleteCurrentParams,
  type AutocompleteStatus,
  type CommandResponse,
  type ConfigSnapshot,
  isTauri,
  openhumanAutocompleteAccept,
  openhumanAutocompleteClearHistory,
  openhumanAutocompleteCurrent,
  openhumanAutocompleteDebugFocus,
  openhumanAutocompleteHistory,
  openhumanAutocompleteSetStyle,
  openhumanAutocompleteStart,
  openhumanAutocompleteStatus,
  openhumanAutocompleteStop,
  openhumanGetConfig,
} from '../../../../utils/tauriCommands';
import AutocompletePanel from '../AutocompletePanel';

vi.mock('../../../../utils/tauriCommands', () => ({
  isTauri: vi.fn(() => true),
  openhumanAutocompleteAccept: vi.fn(),
  openhumanAutocompleteClearHistory: vi.fn(),
  openhumanAutocompleteCurrent: vi.fn(),
  openhumanAutocompleteDebugFocus: vi.fn(),
  openhumanAutocompleteHistory: vi.fn(),
  openhumanAutocompleteSetStyle: vi.fn(),
  openhumanAutocompleteStart: vi.fn(),
  openhumanAutocompleteStatus: vi.fn(),
  openhumanAutocompleteStop: vi.fn(),
  openhumanGetConfig: vi.fn(),
}));

type RuntimeHarness = { status: AutocompleteStatus; config: AutocompleteConfig };

const makeConfigSnapshot = (config: AutocompleteConfig): CommandResponse<ConfigSnapshot> => ({
  result: {
    config: { autocomplete: config },
    workspace_dir: '/tmp/openhuman-e2e',
    config_path: '/tmp/openhuman-e2e/config.toml',
  },
  logs: [],
});

const cloneStatus = (status: AutocompleteStatus): AutocompleteStatus => ({
  ...status,
  suggestion: status.suggestion ? { ...status.suggestion } : null,
});

describe('AutocompletePanel', () => {
  let runtime: RuntimeHarness;

  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(isTauri).mockReturnValue(true);

    runtime = {
      status: {
        platform_supported: true,
        enabled: true,
        running: false,
        phase: 'idle',
        debounce_ms: 120,
        model_id: 'gemma3:4b-it-qat',
        app_name: 'OpenHuman',
        last_error: null,
        updated_at_ms: Date.now(),
        suggestion: null,
      },
      config: {
        enabled: true,
        debounce_ms: 120,
        max_chars: 384,
        style_preset: 'balanced',
        style_instructions: null,
        style_examples: [],
        disabled_apps: [],
        accept_with_tab: true,
        overlay_ttl_ms: 1100,
      },
    };

    vi.mocked(openhumanAutocompleteStatus).mockImplementation(async () => ({
      result: cloneStatus(runtime.status),
      logs: [
        `[autocomplete] status running=${runtime.status.running ? 'yes' : 'no'} phase=${runtime.status.phase}`,
      ],
    }));

    vi.mocked(openhumanGetConfig).mockImplementation(async () =>
      makeConfigSnapshot(runtime.config)
    );

    vi.mocked(openhumanAutocompleteHistory).mockResolvedValue({
      result: { entries: [] },
      logs: ['[autocomplete] history entries=0'],
    });

    vi.mocked(openhumanAutocompleteClearHistory).mockResolvedValue({
      result: { cleared: 0 },
      logs: ['[autocomplete] history cleared=0'],
    });

    vi.mocked(openhumanAutocompleteSetStyle).mockImplementation(async params => {
      runtime.config = {
        ...runtime.config,
        ...params,
        style_instructions: params.style_instructions ?? runtime.config.style_instructions,
        style_examples: params.style_examples ?? runtime.config.style_examples,
        disabled_apps: params.disabled_apps ?? runtime.config.disabled_apps,
      };
      runtime.status.enabled = runtime.config.enabled;
      runtime.status.debounce_ms = runtime.config.debounce_ms;
      if (!runtime.config.enabled) {
        runtime.status.running = false;
        runtime.status.phase = 'disabled';
        runtime.status.suggestion = null;
      }
      return {
        result: { config: { ...runtime.config } },
        logs: [
          `[autocomplete] set_style enabled=${String(runtime.config.enabled)} debounce=${String(runtime.config.debounce_ms)} max_chars=${String(runtime.config.max_chars)} accept_with_tab=${String(runtime.config.accept_with_tab)}`,
        ],
      };
    });

    vi.mocked(openhumanAutocompleteStart).mockImplementation(async params => {
      if (!runtime.config.enabled) {
        return { result: { started: false }, logs: ['[autocomplete] start blocked: disabled'] };
      }
      runtime.status.running = true;
      runtime.status.phase = 'idle';
      runtime.status.debounce_ms = params?.debounce_ms ?? runtime.config.debounce_ms;
      runtime.status.updated_at_ms = Date.now();
      return {
        result: { started: true },
        logs: [`[autocomplete] start running=yes debounce=${String(runtime.status.debounce_ms)}`],
      };
    });

    vi.mocked(openhumanAutocompleteStop).mockImplementation(async () => {
      runtime.status.running = false;
      runtime.status.phase = 'idle';
      runtime.status.suggestion = null;
      runtime.status.updated_at_ms = Date.now();
      return { result: { stopped: true }, logs: ['[autocomplete] stop running=no'] };
    });

    vi.mocked(openhumanAutocompleteCurrent).mockImplementation(
      async (params?: AutocompleteCurrentParams) => {
        const context = params?.context?.trim() ?? '';
        const suggestion = context ? 'completion draft' : '';
        runtime.status.app_name = 'OpenHuman';
        runtime.status.phase = suggestion ? 'ready' : 'idle';
        runtime.status.suggestion = suggestion ? { value: suggestion, confidence: 0.74 } : null;
        runtime.status.updated_at_ms = Date.now();
        return {
          result: {
            app_name: runtime.status.app_name,
            context,
            suggestion: runtime.status.suggestion,
          },
          logs: [
            `[autocomplete] current context_chars=${String(context.length)} suggestion=${suggestion ? 'yes' : 'no'}`,
          ],
        };
      }
    );

    vi.mocked(openhumanAutocompleteAccept).mockImplementation(async params => {
      const value = params?.suggestion ?? runtime.status.suggestion?.value ?? '';
      if (!value) {
        return {
          result: {
            accepted: false,
            applied: false,
            value: null,
            reason: 'no suggestion available',
          },
          logs: ['[autocomplete] accept no-op'],
        };
      }
      runtime.status.phase = 'idle';
      runtime.status.suggestion = null;
      runtime.status.updated_at_ms = Date.now();
      return {
        result: { accepted: true, applied: true, value, reason: null },
        logs: [`[autocomplete] accept applied value_chars=${String(value.length)}`],
      };
    });

    vi.mocked(openhumanAutocompleteDebugFocus).mockResolvedValue({
      result: {
        app_name: 'OpenHuman',
        role: 'TextArea',
        context: 'draft context',
        selected_text: null,
        raw_error: null,
      },
      logs: ['[autocomplete] debug focus'],
    });
  });

  it('runs start → suggest → accept flow and reflects status, settings, and logs', async () => {
    renderWithProviders(<AutocompletePanel />, { initialEntries: ['/settings/autocomplete'] });

    await screen.findByText('Inline Autocomplete');
    await waitFor(() => {
      expect(screen.getByText('Phase: idle')).toBeInTheDocument();
    });

    expect(screen.getByText('Platform supported: yes')).toBeInTheDocument();
    expect(screen.getByText('Running: no')).toBeInTheDocument();
    expect(screen.getByText('Debounce: 120ms')).toBeInTheDocument();

    const debounceRow = screen.getByText('Debounce (ms)').closest('label');
    const debounceInput = debounceRow?.querySelector('input') as HTMLInputElement;
    fireEvent.change(debounceInput, { target: { value: '220' } });

    const maxCharsRow = screen.getByText('Max Chars').closest('label');
    const maxCharsInput = maxCharsRow?.querySelector('input') as HTMLInputElement;
    fireEvent.change(maxCharsInput, { target: { value: '256' } });

    const acceptWithTabRow = screen.getByText('Accept With Tab').closest('label');
    const acceptWithTabInput = acceptWithTabRow?.querySelector('input') as HTMLInputElement;
    fireEvent.click(acceptWithTabInput);

    fireEvent.click(screen.getByRole('button', { name: 'Save Autocomplete Settings' }));

    await waitFor(() => {
      expect(openhumanAutocompleteSetStyle).toHaveBeenCalledWith(
        expect.objectContaining({ debounce_ms: 220, max_chars: 256, accept_with_tab: false })
      );
    });

    fireEvent.click(screen.getByRole('button', { name: 'Start' }));

    await waitFor(() => {
      expect(openhumanAutocompleteStart).toHaveBeenCalled();
    });
    await waitFor(() => {
      expect(screen.getByText('Running: yes')).toBeInTheDocument();
      expect(screen.getByText('Debounce: 220ms')).toBeInTheDocument();
    });

    const contextLabel = screen.getByText('Context Override (optional)');
    const contextInput = contextLabel.parentElement?.querySelector(
      'textarea'
    ) as HTMLTextAreaElement;
    fireEvent.change(contextInput, { target: { value: 'Please review this change' } });
    fireEvent.click(screen.getByRole('button', { name: 'Get Suggestion' }));

    await waitFor(() => {
      expect(openhumanAutocompleteCurrent).toHaveBeenCalledWith({
        context: 'Please review this change',
      });
    });
    await waitFor(() => {
      expect(screen.getByText('Current suggestion: completion draft')).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole('button', { name: 'Accept Suggestion' }));

    await waitFor(() => {
      expect(openhumanAutocompleteAccept).toHaveBeenCalledWith({
        suggestion: 'completion draft',
        skip_apply: true,
      });
    });
    await waitFor(() => {
      expect(screen.getByText('Current suggestion: none')).toBeInTheDocument();
      expect(screen.getByText('Accepted: completion draft')).toBeInTheDocument();
    });

    const logsSection = screen.getByText('Live Logs').closest('section');
    expect(logsSection).not.toBeNull();
    const logsScope = within(logsSection as HTMLElement);
    const logsOutput = (logsSection as HTMLElement).querySelector('pre') as HTMLElement;

    expect(logsOutput.textContent).toContain('[autocomplete] start');
    expect(logsOutput.textContent).toContain('[autocomplete] current');
    expect(logsOutput.textContent).toContain('phase idle -> ready');
    expect(logsOutput.textContent).toContain('phase ready -> idle');

    fireEvent.click(logsScope.getByRole('button', { name: 'Clear' }));
    await waitFor(() => {
      expect(logsOutput.textContent).toContain('No logs yet.');
    });
  });
});
