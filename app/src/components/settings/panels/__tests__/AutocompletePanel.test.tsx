import { fireEvent, screen, waitFor, within } from '@testing-library/react';
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

    await screen.findByText('Style Preset');

    // Verify user-facing controls are present
    expect(screen.getByText('Enabled')).toBeInTheDocument();
    expect(screen.getByText('Accept With Tab')).toBeInTheDocument();
    expect(screen.getByText('Style Preset')).toBeInTheDocument();

    // Verify runtime status section shows
    await waitFor(() => {
      expect(screen.getByText('Running: No')).toBeInTheDocument();
    });

    // Change style preset and save using the labeled select
    const presetSelect = screen.getByRole('combobox', { name: 'Style Preset' });
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

    await screen.findByText('Style Preset');

    // Wait for status to load
    await waitFor(() => {
      expect(screen.getByText('Running: No')).toBeInTheDocument();
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

    await screen.findByText('Style Preset');

    // Wait for config to load
    await waitFor(() => {
      expect(screen.getByText('Running: No')).toBeInTheDocument();
    });

    // Toggle enabled off via SettingsSwitch (role="switch")
    const enabledSwitch = screen.getByRole('switch', { name: 'Enabled' });
    fireEvent.click(enabledSwitch);

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

    await screen.findByText('Style Preset');
    expect(screen.getByText('Advanced settings')).toBeInTheDocument();
  });

  it('seeds the tuning inputs from config and saves edited values', async () => {
    runtime.config.debounce_ms = 500;
    runtime.config.max_chars = 800;
    runtime.config.overlay_ttl_ms = 2000;

    renderWithProviders(<AutocompletePanel />, { initialEntries: ['/settings/autocomplete'] });

    await screen.findByText('Style Preset');

    // SettingsNumberField wraps the input in a div; find the inner spinbutton
    const debounceWrapper = await screen.findByTestId('autocomplete-debounce-ms');
    const maxCharsWrapper = screen.getByTestId('autocomplete-max-chars');
    const overlayTtlWrapper = screen.getByTestId('autocomplete-overlay-ttl-ms');

    const debounce = within(debounceWrapper).getByRole('spinbutton') as HTMLInputElement;
    const maxChars = within(maxCharsWrapper).getByRole('spinbutton') as HTMLInputElement;
    const overlayTtl = within(overlayTtlWrapper).getByRole('spinbutton') as HTMLInputElement;

    // Seeded from loaded config.
    await waitFor(() => expect(debounce.value).toBe('500'));
    expect(maxChars.value).toBe('800');
    expect(overlayTtl.value).toBe('2000');

    // Edit and save.
    fireEvent.change(debounce, { target: { value: '250' } });
    fireEvent.change(maxChars, { target: { value: '512' } });
    fireEvent.change(overlayTtl, { target: { value: '900' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save Settings' }));

    await waitFor(() => {
      expect(openhumanAutocompleteSetStyle).toHaveBeenCalledWith(
        expect.objectContaining({ debounce_ms: 250, max_chars: 512, overlay_ttl_ms: 900 })
      );
    });
  });

  it('allows clearing a tuning field mid-edit and clamps to safe minimums at save', async () => {
    renderWithProviders(<AutocompletePanel />, { initialEntries: ['/settings/autocomplete'] });

    await screen.findByText('Style Preset');

    const maxCharsWrapper = await screen.findByTestId('autocomplete-max-chars');
    const debounceWrapper = screen.getByTestId('autocomplete-debounce-ms');

    const maxChars = within(maxCharsWrapper).getByRole('spinbutton') as HTMLInputElement;
    const debounce = within(debounceWrapper).getByRole('spinbutton') as HTMLInputElement;

    // Intermediate empty / zero states are preserved while typing (no snap).
    fireEvent.change(maxChars, { target: { value: '' } });
    expect(maxChars.value).toBe('');
    fireEvent.change(maxChars, { target: { value: '0' } });
    expect(maxChars.value).toBe('0');
    fireEvent.change(debounce, { target: { value: '' } });
    expect(debounce.value).toBe('');

    // Clamping happens at save: max_chars -> >= 1, debounce -> >= 0.
    fireEvent.click(screen.getByRole('button', { name: 'Save Settings' }));
    await waitFor(() => {
      expect(openhumanAutocompleteSetStyle).toHaveBeenCalledWith(
        expect.objectContaining({ max_chars: 384, debounce_ms: 0 })
      );
    });
  });

  // ─── Disabled apps textarea ───────────────────────────────────────────────

  it('seeds the disabled-apps textarea from config and saves changes', async () => {
    runtime.config.disabled_apps = ['Slack', 'Zoom'];

    renderWithProviders(<AutocompletePanel />, { initialEntries: ['/settings/autocomplete'] });
    await screen.findByText('Style Preset');

    const textarea = (await screen.findByRole('textbox', {
      name: /disabled apps/i,
    })) as HTMLTextAreaElement;

    await waitFor(() => expect(textarea.value).toContain('Slack'));
    expect(textarea.value).toContain('Zoom');

    // Edit the textarea
    fireEvent.change(textarea, { target: { value: 'Teams\nDiscord' } });
    expect(textarea.value).toBe('Teams\nDiscord');

    // Save includes updated disabled_apps
    fireEvent.click(screen.getByRole('button', { name: 'Save Settings' }));
    await waitFor(() => {
      expect(openhumanAutocompleteSetStyle).toHaveBeenCalledWith(
        expect.objectContaining({ disabled_apps: ['Teams', 'Discord'] })
      );
    });
  });

  // ─── onCommit callbacks on number fields ──────────────────────────────────

  it('committing a debounce value via Enter triggers saveConfig', async () => {
    renderWithProviders(<AutocompletePanel />, { initialEntries: ['/settings/autocomplete'] });
    await screen.findByText('Style Preset');

    const debounceWrapper = await screen.findByTestId('autocomplete-debounce-ms');
    const debounce = within(debounceWrapper).getByRole('spinbutton') as HTMLInputElement;

    fireEvent.change(debounce, { target: { value: '300' } });
    // onCommit fires when SettingsNumberField calls its onCommit prop; simulate by
    // pressing Enter which SettingsNumberField forwards to onCommit.
    fireEvent.keyDown(debounce, { key: 'Enter' });

    await waitFor(() => {
      expect(openhumanAutocompleteSetStyle).toHaveBeenCalledWith(
        expect.objectContaining({ debounce_ms: 300 })
      );
    });
  });

  it('committing a max-chars value triggers saveConfig', async () => {
    renderWithProviders(<AutocompletePanel />, { initialEntries: ['/settings/autocomplete'] });
    await screen.findByText('Style Preset');

    const maxCharsWrapper = await screen.findByTestId('autocomplete-max-chars');
    const maxChars = within(maxCharsWrapper).getByRole('spinbutton') as HTMLInputElement;

    fireEvent.change(maxChars, { target: { value: '512' } });
    fireEvent.keyDown(maxChars, { key: 'Enter' });

    await waitFor(() => {
      expect(openhumanAutocompleteSetStyle).toHaveBeenCalledWith(
        expect.objectContaining({ max_chars: 512 })
      );
    });
  });

  it('committing an overlay-ttl value triggers saveConfig', async () => {
    renderWithProviders(<AutocompletePanel />, { initialEntries: ['/settings/autocomplete'] });
    await screen.findByText('Style Preset');

    const overlayWrapper = await screen.findByTestId('autocomplete-overlay-ttl-ms');
    const overlayTtl = within(overlayWrapper).getByRole('spinbutton') as HTMLInputElement;

    fireEvent.change(overlayTtl, { target: { value: '2500' } });
    fireEvent.keyDown(overlayTtl, { key: 'Enter' });

    await waitFor(() => {
      expect(openhumanAutocompleteSetStyle).toHaveBeenCalledWith(
        expect.objectContaining({ overlay_ttl_ms: 2500 })
      );
    });
  });

  // ─── didNotStart branch ───────────────────────────────────────────────────

  it('shows "did not start" message when autocomplete start returns started=false', async () => {
    vi.mocked(openhumanAutocompleteStart).mockResolvedValueOnce({
      result: { started: false },
      logs: [],
    });

    renderWithProviders(<AutocompletePanel />, { initialEntries: ['/settings/autocomplete'] });
    await screen.findByText('Style Preset');

    await waitFor(() => expect(screen.getByText('Running: No')).toBeInTheDocument());

    fireEvent.click(screen.getByRole('button', { name: 'Start' }));
    await waitFor(() => expect(openhumanAutocompleteStart).toHaveBeenCalled());

    await waitFor(() =>
      expect(screen.getByText(/did not start|autocomplete did not/i)).toBeInTheDocument()
    );
  });
});
