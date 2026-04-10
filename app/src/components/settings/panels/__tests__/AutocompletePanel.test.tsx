import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import {
  type AutocompleteConfig,
  type AutocompleteStatus,
  type CommandResponse,
  type ConfigSnapshot,
  isTauri,
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

describe('AutocompletePanel (simplified)', () => {
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
      logs: [],
    }));

    vi.mocked(openhumanGetConfig).mockImplementation(async () =>
      makeConfigSnapshot(runtime.config)
    );

    vi.mocked(openhumanAutocompleteSetStyle).mockImplementation(async params => {
      runtime.config = {
        ...runtime.config,
        ...params,
        style_instructions: params.style_instructions ?? runtime.config.style_instructions,
        style_examples: params.style_examples ?? runtime.config.style_examples,
        disabled_apps: params.disabled_apps ?? runtime.config.disabled_apps,
      };
      runtime.status.enabled = runtime.config.enabled;
      return { result: { config: { ...runtime.config } }, logs: [] };
    });

    vi.mocked(openhumanAutocompleteStart).mockImplementation(async () => {
      if (!runtime.config.enabled) {
        return { result: { started: false }, logs: [] };
      }
      runtime.status.running = true;
      runtime.status.phase = 'idle';
      return { result: { started: true }, logs: [] };
    });

    vi.mocked(openhumanAutocompleteStop).mockImplementation(async () => {
      runtime.status.running = false;
      runtime.status.phase = 'idle';
      runtime.status.suggestion = null;
      return { result: { stopped: true }, logs: [] };
    });
  });

  it('shows user-facing settings and can save style preset changes', async () => {
    renderWithProviders(<AutocompletePanel />, { initialEntries: ['/settings/autocomplete'] });

    await screen.findByText('Autocomplete');

    // Verify user-facing controls are present
    expect(screen.getByText('Enabled')).toBeInTheDocument();
    expect(screen.getByText('Accept With Tab')).toBeInTheDocument();
    expect(screen.getByText('Style Preset')).toBeInTheDocument();

    // Verify runtime status section shows
    await waitFor(() => {
      expect(screen.getByText('Running: no')).toBeInTheDocument();
    });

    // Change style preset and save
    const presetRow = screen.getByText('Style Preset').closest('label');
    const presetSelect = presetRow?.querySelector('select') as HTMLSelectElement;
    fireEvent.change(presetSelect, { target: { value: 'concise' } });

    fireEvent.click(screen.getByRole('button', { name: 'Save Settings' }));

    await waitFor(() => {
      expect(openhumanAutocompleteSetStyle).toHaveBeenCalledWith(
        expect.objectContaining({ style_preset: 'concise', accept_with_tab: true })
      );
    });

    expect(await screen.findByText('Autocomplete settings saved.')).toBeInTheDocument();
  });

  it('can start and stop the autocomplete runtime', async () => {
    renderWithProviders(<AutocompletePanel />, { initialEntries: ['/settings/autocomplete'] });

    await screen.findByText('Autocomplete');

    // Wait for status to load
    await waitFor(() => {
      expect(screen.getByText('Running: no')).toBeInTheDocument();
    });

    // Start
    fireEvent.click(screen.getByRole('button', { name: 'Start' }));
    await waitFor(() => {
      expect(openhumanAutocompleteStart).toHaveBeenCalled();
    });
    await waitFor(() => {
      expect(screen.getByText('Autocomplete started.')).toBeInTheDocument();
    });

    // Stop
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }));
    await waitFor(() => {
      expect(openhumanAutocompleteStop).toHaveBeenCalled();
    });
    await waitFor(() => {
      expect(screen.getByText('Autocomplete stopped.')).toBeInTheDocument();
    });
  });

  it('preserves advanced settings when saving from the simplified panel', async () => {
    runtime.config.debounce_ms = 500;
    runtime.config.max_chars = 800;
    runtime.config.overlay_ttl_ms = 2000;

    renderWithProviders(<AutocompletePanel />, { initialEntries: ['/settings/autocomplete'] });

    await screen.findByText('Autocomplete');

    // Wait for config to load
    await waitFor(() => {
      expect(screen.getByText('Running: no')).toBeInTheDocument();
    });

    // Toggle enabled off and save
    const enabledLabel = screen.getByText('Enabled').closest('label');
    const enabledCheckbox = enabledLabel?.querySelector(
      'input[type="checkbox"]'
    ) as HTMLInputElement;
    fireEvent.click(enabledCheckbox);

    fireEvent.click(screen.getByRole('button', { name: 'Save Settings' }));

    await waitFor(() => {
      expect(openhumanAutocompleteSetStyle).toHaveBeenCalledWith(
        expect.objectContaining({
          enabled: false,
          debounce_ms: 500,
          max_chars: 800,
          overlay_ttl_ms: 2000,
        })
      );
    });
  });

  it('shows the Advanced settings link', async () => {
    renderWithProviders(<AutocompletePanel />, { initialEntries: ['/settings/autocomplete'] });

    await screen.findByText('Autocomplete');
    expect(screen.getByText('Advanced settings')).toBeInTheDocument();
  });
});
