/**
 * AutocompleteDebugPanel coverage tests.
 *
 * Target uncovered lines (from diff-cover report):
 * 497,501,505,509,513,517,521,525,529,537,539,547-548,555,574,584,591,594,598,
 * 618,639,655,671,682,693,702,704,719-721,727,729,731,737,739-740,747
 *
 * The i18n mock returns the full key as-is (key-passthrough).
 * Status values are rendered inline with labels; use container queries or
 * partial-text selectors for split-text-node elements.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// ---------------------------------------------------------------------------
// Module mocks
// ---------------------------------------------------------------------------

const {
  mockIsTauri,
  mockAutocompleteStatus,
  mockGetConfig,
  mockAutocompleteStart,
  mockAutocompleteStop,
  mockAutocompleteCurrent,
  mockAutocompleteAccept,
  mockAutocompleteDebugFocus,
  mockAutocompleteSetStyle,
  mockAutocompleteHistory,
  mockAutocompleteClearHistory,
} = vi.hoisted(() => ({
  mockIsTauri: vi.fn(() => true),
  mockAutocompleteStatus: vi.fn(),
  mockGetConfig: vi.fn(),
  mockAutocompleteStart: vi.fn(),
  mockAutocompleteStop: vi.fn(),
  mockAutocompleteCurrent: vi.fn(),
  mockAutocompleteAccept: vi.fn(),
  mockAutocompleteDebugFocus: vi.fn(),
  mockAutocompleteSetStyle: vi.fn(),
  mockAutocompleteHistory: vi.fn(),
  mockAutocompleteClearHistory: vi.fn(),
}));

vi.mock('../../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: vi.fn(), breadcrumbs: [] }),
}));

vi.mock('../components/SettingsHeader', () => ({ default: () => null }));

vi.mock('../../../../utils/tauriCommands', () => ({
  isTauri: mockIsTauri,
  openhumanAutocompleteStatus: mockAutocompleteStatus,
  openhumanGetConfig: mockGetConfig,
  openhumanAutocompleteStart: mockAutocompleteStart,
  openhumanAutocompleteStop: mockAutocompleteStop,
  openhumanAutocompleteCurrent: mockAutocompleteCurrent,
  openhumanAutocompleteAccept: mockAutocompleteAccept,
  openhumanAutocompleteDebugFocus: mockAutocompleteDebugFocus,
  openhumanAutocompleteSetStyle: mockAutocompleteSetStyle,
  openhumanAutocompleteHistory: mockAutocompleteHistory,
  openhumanAutocompleteClearHistory: mockAutocompleteClearHistory,
}));

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/**
 * Assert that text appears anywhere in the document body.
 * Use for status-row values that are rendered as bare text nodes inside a div
 * that also contains the label and colon — getByText can't isolate those.
 */
const expectInBody = (text: string) => expect(document.body.textContent).toContain(text);

const baseStatus = () => ({
  phase: 'idle',
  running: false,
  enabled: true,
  platform_supported: true,
  debounce_ms: 120,
  model_id: 'claude-haiku',
  app_name: 'TextEdit',
  last_error: null,
  suggestion: null,
});

const baseConfig = () => ({
  result: {
    config: {
      autocomplete: {
        enabled: true,
        debounce_ms: 150,
        max_chars: 400,
        style_preset: 'balanced',
        style_instructions: 'be concise',
        style_examples: ['example one', 'example two'],
        disabled_apps: [],
        accept_with_tab: true,
        overlay_ttl_ms: 1100,
      },
    },
  },
  logs: [],
});

const statusResponse = (overrides: Record<string, unknown> = {}) => ({
  result: { ...baseStatus(), ...overrides },
  logs: ['[runtime] phase=idle'],
});

async function renderPanel() {
  const { default: AutocompleteDebugPanel } = await import('../AutocompleteDebugPanel');
  return render(<AutocompleteDebugPanel />);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('AutocompleteDebugPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockIsTauri.mockReturnValue(true);
    mockAutocompleteStatus.mockResolvedValue(statusResponse());
    mockGetConfig.mockResolvedValue(baseConfig());
    mockAutocompleteHistory.mockResolvedValue({ result: { entries: [] }, logs: [] });
    mockAutocompleteStart.mockResolvedValue({ result: { started: true }, logs: [] });
    mockAutocompleteStop.mockResolvedValue({ result: {}, logs: [] });
    mockAutocompleteCurrent.mockResolvedValue({
      result: { suggestion: { value: 'hello world' } },
      logs: [],
    });
    mockAutocompleteAccept.mockResolvedValue({
      result: { accepted: true, value: 'hello world' },
      logs: [],
    });
    mockAutocompleteDebugFocus.mockResolvedValue({
      result: { app_name: 'TestApp', role: 'textField', context: 'some text here' },
      logs: [],
    });
    mockAutocompleteSetStyle.mockResolvedValue({
      result: {
        config: {
          debounce_ms: 150,
          max_chars: 400,
          overlay_ttl_ms: 1100,
          style_instructions: 'be concise',
          style_examples: ['example one'],
        },
      },
      logs: [],
    });
    mockAutocompleteClearHistory.mockResolvedValue(undefined);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  // ── Runtime section: section renders (lines 492–530) ──────────────────────

  it('renders runtime section heading (line 492)', async () => {
    await renderPanel();
    // The SettingsSection title is a unique text node, not split
    await waitFor(() =>
      expect(screen.getByText('settings.autocomplete.appFilter.runtime')).toBeInTheDocument()
    );
  });

  it('renders platform_supported row with yes value (lines 496-498)', async () => {
    await renderPanel();
    // Status values are text nodes inside a div that also holds the label.
    // Use body.textContent checks inside waitFor to avoid split-node matching issues.
    await waitFor(() => {
      expectInBody('settings.autocomplete.appFilter.platformSupported');
      expectInBody('common.yes');
    });
  });

  it('renders common.no for false values (lines 497, 501, 505)', async () => {
    mockAutocompleteStatus.mockResolvedValue(
      statusResponse({ platform_supported: false, running: false, enabled: false })
    );
    await renderPanel();
    await waitFor(() => {
      expectInBody('common.no');
    });
  });

  it('renders phase value from status (line 509)', async () => {
    mockAutocompleteStatus.mockResolvedValue(statusResponse({ phase: 'composing' }));
    await renderPanel();
    await waitFor(() => expectInBody('composing'));
  });

  it('renders model_id from status (line 517)', async () => {
    mockAutocompleteStatus.mockResolvedValue(statusResponse({ model_id: 'gpt-4o' }));
    await renderPanel();
    await waitFor(() => expectInBody('gpt-4o'));
  });

  it('shows notApplicable when model_id is null (line 517)', async () => {
    mockAutocompleteStatus.mockResolvedValue(statusResponse({ model_id: null }));
    await renderPanel();
    await waitFor(() => expectInBody('settings.autocomplete.shared.notApplicable'));
  });

  it('renders app_name from status (line 521)', async () => {
    mockAutocompleteStatus.mockResolvedValue(statusResponse({ app_name: 'CodeEditor' }));
    await renderPanel();
    await waitFor(() => expectInBody('CodeEditor'));
  });

  it('renders last_error value from status (line 525)', async () => {
    mockAutocompleteStatus.mockResolvedValue(statusResponse({ last_error: 'context_limit' }));
    await renderPanel();
    await waitFor(() => expectInBody('context_limit'));
  });

  it('shows none when last_error is null (line 525)', async () => {
    mockAutocompleteStatus.mockResolvedValue(statusResponse({ last_error: null }));
    await renderPanel();
    // none appears for both last_error=null and suggestion=null
    await waitFor(() => expectInBody('settings.autocomplete.shared.none'));
  });

  it('renders suggestion value from status (line 529)', async () => {
    mockAutocompleteStatus.mockResolvedValue(
      statusResponse({ suggestion: { value: 'suggested completion text' } })
    );
    await renderPanel();
    await waitFor(() => expectInBody('suggested completion text'));
  });

  // ── Refresh status button (line 537) ──────────────────────────────────────

  it('calls refreshStatus when Refresh Status button clicked (line 537)', async () => {
    await renderPanel();
    await waitFor(() => screen.getByText('settings.autocomplete.appFilter.refreshStatus'));

    fireEvent.click(screen.getByText('settings.autocomplete.appFilter.refreshStatus'));

    await waitFor(() => expect(mockAutocompleteStatus).toHaveBeenCalledTimes(2));
  });

  // ── Start / Stop buttons (lines 539, 547-548, 555) ────────────────────────

  it('calls start when Start button clicked (line 547)', async () => {
    await renderPanel();
    await waitFor(() => screen.getByText('autocomplete.start'));
    fireEvent.click(screen.getByText('autocomplete.start'));
    await waitFor(() => expect(mockAutocompleteStart).toHaveBeenCalled());
  });

  it('shows autocomplete.started after successful start', async () => {
    await renderPanel();
    await waitFor(() => screen.getByText('autocomplete.start'));
    fireEvent.click(screen.getByText('autocomplete.start'));
    await waitFor(() => expect(screen.getByText('autocomplete.started')).toBeInTheDocument());
  });

  it('calls stop when Stop button clicked (line 555)', async () => {
    mockAutocompleteStatus.mockResolvedValue(statusResponse({ running: true }));
    await renderPanel();
    await waitFor(() => screen.getByText('autocomplete.stop'));
    fireEvent.click(screen.getByText('autocomplete.stop'));
    await waitFor(() => expect(mockAutocompleteStop).toHaveBeenCalled());
  });

  it('shows alreadyRunning when start returns not-started (line 263)', async () => {
    mockAutocompleteStart.mockResolvedValue({ result: { started: false }, logs: [] });
    // refreshStatus after start will show running=true
    let callCount = 0;
    mockAutocompleteStatus.mockImplementation(() => {
      callCount++;
      return Promise.resolve(statusResponse({ running: callCount > 1, enabled: true }));
    });

    await renderPanel();
    await waitFor(() => screen.getByText('autocomplete.start'));
    fireEvent.click(screen.getByText('autocomplete.start'));

    await waitFor(() => expectInBody('settings.autocomplete.debug.alreadyRunning'));
  });

  it('shows disabledInSettings when start fails and enabled=false (line 261)', async () => {
    mockAutocompleteStart.mockResolvedValue({ result: { started: false }, logs: [] });
    let callCount = 0;
    mockAutocompleteStatus.mockImplementation(() => {
      callCount++;
      // After first load: enabled=true. After start attempt: enabled=false
      return Promise.resolve(statusResponse({ running: false, enabled: callCount === 1 }));
    });

    await renderPanel();
    await waitFor(() => screen.getByText('autocomplete.start'));
    fireEvent.click(screen.getByText('autocomplete.start'));

    await waitFor(() => expectInBody('settings.autocomplete.debug.disabledInSettings'));
  });

  // ── Test section: getSuggestion (lines 574, 584) ──────────────────────────

  it('getSuggestion → shows suggestionPrefix (line 574)', async () => {
    await renderPanel();
    await waitFor(() => screen.getByText('settings.autocomplete.appFilter.getSuggestion'));
    fireEvent.click(screen.getByText('settings.autocomplete.appFilter.getSuggestion'));
    await waitFor(() =>
      expect(screen.getByText('settings.autocomplete.debug.suggestionPrefix')).toBeInTheDocument()
    );
  });

  it('getSuggestion with null result → shows noSuggestionReturned (line 584)', async () => {
    mockAutocompleteCurrent.mockResolvedValue({ result: { suggestion: null }, logs: [] });
    await renderPanel();
    await waitFor(() => screen.getByText('settings.autocomplete.appFilter.getSuggestion'));
    fireEvent.click(screen.getByText('settings.autocomplete.appFilter.getSuggestion'));
    await waitFor(() =>
      expect(
        screen.getByText('settings.autocomplete.debug.noSuggestionReturned')
      ).toBeInTheDocument()
    );
  });

  // ── Test section: acceptSuggestion (lines 591, 594) ──────────────────────

  it('acceptSuggestion accepted=true → shows acceptedPrefix (line 591)', async () => {
    await renderPanel();
    await waitFor(() => screen.getByText('settings.autocomplete.appFilter.acceptSuggestion'));
    fireEvent.click(screen.getByText('settings.autocomplete.appFilter.acceptSuggestion'));
    await waitFor(() =>
      expect(screen.getByText('settings.autocomplete.debug.acceptedPrefix')).toBeInTheDocument()
    );
  });

  it('acceptSuggestion accepted=false with reason → shows reason (line 594)', async () => {
    mockAutocompleteAccept.mockResolvedValue({
      result: { accepted: false, reason: 'no suggestion pending', value: null },
      logs: [],
    });
    await renderPanel();
    await waitFor(() => screen.getByText('settings.autocomplete.appFilter.acceptSuggestion'));
    fireEvent.click(screen.getByText('settings.autocomplete.appFilter.acceptSuggestion'));
    await waitFor(() => expect(screen.getByText('no suggestion pending')).toBeInTheDocument());
  });

  it('acceptSuggestion accepted=false with no reason → noSuggestionApplied (line 594)', async () => {
    mockAutocompleteAccept.mockResolvedValue({
      result: { accepted: false, reason: null, value: null },
      logs: [],
    });
    await renderPanel();
    await waitFor(() => screen.getByText('settings.autocomplete.appFilter.acceptSuggestion'));
    fireEvent.click(screen.getByText('settings.autocomplete.appFilter.acceptSuggestion'));
    await waitFor(() =>
      expect(
        screen.getByText('settings.autocomplete.debug.noSuggestionApplied')
      ).toBeInTheDocument()
    );
  });

  // ── debugFocus (line 598) ──────────────────────────────────────────────────

  it('debugFocus renders JSON pre with app_name content (line 598)', async () => {
    await renderPanel();
    await waitFor(() => screen.getByText('settings.autocomplete.appFilter.debugFocus'));
    fireEvent.click(screen.getByText('settings.autocomplete.appFilter.debugFocus'));
    // JSON.stringify result is rendered in a <pre>; check body textContent
    await waitFor(() => expectInBody('"app_name"'));
  });

  it('debugFocus null result → focusDebug shows null JSON (line 598)', async () => {
    mockAutocompleteDebugFocus.mockResolvedValue({ result: null, logs: [] });
    await renderPanel();
    await waitFor(() => screen.getByText('settings.autocomplete.appFilter.debugFocus'));
    fireEvent.click(screen.getByText('settings.autocomplete.appFilter.debugFocus'));
    await waitFor(() => expect(mockAutocompleteDebugFocus).toHaveBeenCalled());
    // null result → focusDebug = "null", no app_name key in output
    await waitFor(() => expect(document.body.textContent).not.toContain('"app_name"'));
  });

  // ── Live Logs section (line 618) ──────────────────────────────────────────

  it('renders noLogs placeholder before any action (line 618)', async () => {
    await renderPanel();
    await waitFor(() =>
      expect(screen.getByText('settings.autocomplete.appFilter.noLogs')).toBeInTheDocument()
    );
  });

  it('clear logs button empties log display (line 618)', async () => {
    await renderPanel();
    // Perform an action to produce logs
    await waitFor(() => screen.getByText('settings.autocomplete.appFilter.getSuggestion'));
    fireEvent.click(screen.getByText('settings.autocomplete.appFilter.getSuggestion'));
    await waitFor(() => expect(mockAutocompleteCurrent).toHaveBeenCalled());

    fireEvent.click(screen.getByText('common.clear'));
    // After clear, noLogs placeholder appears
    expect(screen.getByText('settings.autocomplete.appFilter.noLogs')).toBeInTheDocument();
  });

  // ── Advanced settings form (lines 639, 655, 671) ──────────────────────────

  it('loads debounce_ms from config (line 639)', async () => {
    await renderPanel();
    await waitFor(() => {
      const input = screen.getByRole('spinbutton', {
        name: 'settings.autocomplete.completionStyle.debounce',
      }) as HTMLInputElement;
      expect(input.value).toBe('150');
    });
  });

  it('loads max_chars from config (line 655)', async () => {
    await renderPanel();
    await waitFor(() => {
      const input = screen.getByRole('spinbutton', {
        name: 'settings.autocomplete.completionStyle.maxChars',
      }) as HTMLInputElement;
      expect(input.value).toBe('400');
    });
  });

  it('loads overlay_ttl_ms from config (line 671)', async () => {
    await renderPanel();
    await waitFor(() => {
      const input = screen.getByRole('spinbutton', {
        name: 'settings.autocomplete.completionStyle.overlayTtl',
      }) as HTMLInputElement;
      expect(input.value).toBe('1100');
    });
  });

  it('updates debounce and calls setStyle on save (lines 682, 693)', async () => {
    await renderPanel();
    await waitFor(() =>
      screen.getByRole('spinbutton', { name: 'settings.autocomplete.completionStyle.debounce' })
    );

    const debounceInput = screen.getByRole('spinbutton', {
      name: 'settings.autocomplete.completionStyle.debounce',
    });
    fireEvent.change(debounceInput, { target: { value: '200' } });
    fireEvent.click(screen.getByText('common.save'));

    await waitFor(() => expect(mockAutocompleteSetStyle).toHaveBeenCalled());
    expect(mockAutocompleteSetStyle).toHaveBeenCalledWith(
      expect.objectContaining({ debounce_ms: 200 })
    );
  });

  it('shows autocomplete.settingsSaved after successful save (line 702)', async () => {
    await renderPanel();
    await waitFor(() => screen.getByText('common.save'));
    fireEvent.click(screen.getByText('common.save'));
    await waitFor(() => expect(screen.getByText('autocomplete.settingsSaved')).toBeInTheDocument());
  });

  // ── History section (lines 719-721, 727, 729, 731, 737, 739-740, 747) ─────

  it('renders noHistory when entries list empty (line 727)', async () => {
    await renderPanel();
    await waitFor(() =>
      expect(
        screen.getByText('settings.autocomplete.completionStyle.noHistory')
      ).toBeInTheDocument()
    );
  });

  it('renders acceptedCompletion (singular) for 1 entry (line 729)', async () => {
    mockAutocompleteHistory.mockResolvedValue({
      result: {
        entries: [
          {
            timestamp_ms: 1700000000000,
            app_name: 'TextEdit',
            context: 'context here',
            suggestion: 'my suggestion',
          },
        ],
      },
      logs: [],
    });
    await renderPanel();
    await waitFor(() =>
      expect(
        screen.getByText('settings.autocomplete.completionStyle.acceptedCompletion')
      ).toBeInTheDocument()
    );
  });

  it('renders acceptedCompletions (plural) and entry rows for 2 entries (lines 731, 737, 739-740)', async () => {
    mockAutocompleteHistory.mockResolvedValue({
      result: {
        entries: [
          {
            timestamp_ms: 1700000000000,
            app_name: 'TestEditor',
            context: 'some context text ending here',
            suggestion: 'suggested completion',
          },
          {
            timestamp_ms: 1700000001000,
            app_name: null,
            context: 'another context',
            suggestion: 'another suggestion',
          },
        ],
      },
      logs: [],
    });
    await renderPanel();
    await waitFor(() =>
      expect(
        screen.getByText('settings.autocomplete.completionStyle.acceptedCompletions')
      ).toBeInTheDocument()
    );
    expect(screen.getByText('suggested completion')).toBeInTheDocument();
    expect(screen.getByText('another suggestion')).toBeInTheDocument();
    expect(screen.getByText('TestEditor')).toBeInTheDocument();
  });

  it('clear history button disabled when entries empty (line 719)', async () => {
    await renderPanel();
    await waitFor(() => screen.getByText('settings.autocomplete.completionStyle.clearHistory'));
    const btn = screen
      .getByText('settings.autocomplete.completionStyle.clearHistory')
      .closest('button') as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
  });

  it('clears history entries on button click (lines 719-721, 747)', async () => {
    mockAutocompleteHistory.mockResolvedValue({
      result: {
        entries: [
          {
            timestamp_ms: 1700000000000,
            app_name: 'TestApp',
            context: 'some context',
            suggestion: 'a suggestion',
          },
        ],
      },
      logs: [],
    });
    await renderPanel();
    await waitFor(() => expect(screen.getByText('a suggestion')).toBeInTheDocument());

    const clearBtn = screen
      .getByText('settings.autocomplete.completionStyle.clearHistory')
      .closest('button') as HTMLButtonElement;
    expect(clearBtn.disabled).toBe(false);

    fireEvent.click(clearBtn);
    await waitFor(() => expect(mockAutocompleteClearHistory).toHaveBeenCalled());
    await waitFor(() =>
      expect(
        screen.getByText('settings.autocomplete.completionStyle.noHistory')
      ).toBeInTheDocument()
    );
  });
});
