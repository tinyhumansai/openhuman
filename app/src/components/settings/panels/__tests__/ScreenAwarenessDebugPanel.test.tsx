/**
 * ScreenAwarenessDebugPanel coverage tests.
 *
 * Target uncovered lines (from diff-cover report):
 * 34,38,41,143,181,193,203,205,245,247,251,255-256,261-262,297
 *
 * These cover:
 * - DebugSection toggle (line 34,38,41): expand/collapse
 * - Config form inputs: baselineFps, use-vision-model checkbox, keep-screenshots (143,181,193)
 * - saveConfig handler (line 203,205)
 * - Vision summaries: list rendering, refresh, empty state (245,247,251,255-256,261-262)
 * - lastError display (297)
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

// ------------------------------------------------------------------
// Module mocks
// ------------------------------------------------------------------

vi.mock('../../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: vi.fn(), breadcrumbs: [] }),
}));

vi.mock('../components/SettingsHeader', () => ({ default: () => null }));

vi.mock('../../../../components/intelligence/ScreenIntelligenceDebugPanel', () => ({
  default: () => <div data-testid="screen-debug-inner">debug-content</div>,
}));

const { mockIsTauri, mockUpdateScreenIntelligenceSettings, mockUseScreenIntelligenceState } =
  vi.hoisted(() => ({
    mockIsTauri: vi.fn(() => true),
    mockUpdateScreenIntelligenceSettings: vi.fn(),
    mockUseScreenIntelligenceState: vi.fn(),
  }));

vi.mock('../../../../utils/tauriCommands', () => ({
  isTauri: mockIsTauri,
  openhumanUpdateScreenIntelligenceSettings: mockUpdateScreenIntelligenceSettings,
}));

vi.mock('../../../../features/screen-intelligence/useScreenIntelligenceState', () => ({
  useScreenIntelligenceState: mockUseScreenIntelligenceState,
}));

// ------------------------------------------------------------------
// Fixture builders
// ------------------------------------------------------------------

const makeStatus = (overrides: Record<string, unknown> = {}) => ({
  platform_supported: true,
  config: {
    enabled: true,
    baseline_fps: 1,
    use_vision_model: true,
    keep_screenshots: false,
    allowlist: ['TextEdit', 'Xcode'],
    denylist: ['Safari'],
    policy_mode: 'all_except_blacklist',
    ...((overrides.config as Record<string, unknown>) ?? {}),
  },
  session: {
    frames_in_memory: 5,
    panic_hotkey: 'Ctrl+Shift+P',
    vision_state: 'running',
    vision_queue_depth: 2,
    last_vision_at_ms: Date.now() - 3000,
  },
  ...overrides,
});

const makeVisionSummary = (overrides: Record<string, unknown> = {}) => ({
  id: 'vs-001',
  captured_at_ms: Date.now() - 10000,
  app_name: 'TextEdit',
  window_title: 'Untitled',
  actionable_notes: 'User is editing a document.',
  ...overrides,
});

const makeDefaultState = (overrides: Record<string, unknown> = {}) => ({
  status: makeStatus(),
  lastError: null,
  isLoadingVision: false,
  recentVisionSummaries: [],
  refreshStatus: vi.fn().mockResolvedValue(null),
  refreshVision: vi.fn().mockResolvedValue([]),
  runCaptureTest: vi.fn().mockResolvedValue(undefined),
  captureTestResult: null,
  isCaptureTestRunning: false,
  ...overrides,
});

async function renderPanel(stateOverrides: Record<string, unknown> = {}) {
  mockUseScreenIntelligenceState.mockReturnValue(makeDefaultState(stateOverrides));
  const { default: ScreenAwarenessDebugPanel } = await import('../ScreenAwarenessDebugPanel');
  return render(<ScreenAwarenessDebugPanel />);
}

// ------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------

describe('ScreenAwarenessDebugPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockIsTauri.mockReturnValue(true);
    mockUpdateScreenIntelligenceSettings.mockResolvedValue({});
  });

  // ── DebugSection expand/collapse (lines 34, 38, 41) ───────────────────────

  it('collapses debug section by default and shows expand label (line 34, 38)', async () => {
    await renderPanel();
    expect(screen.getByText('screenAwareness.debug.expand')).toBeInTheDocument();
    expect(screen.queryByTestId('screen-debug-inner')).not.toBeInTheDocument();
  });

  it('expands debug section when toggle is clicked (line 41)', async () => {
    await renderPanel();

    fireEvent.click(screen.getByText('screenAwareness.debug.debugAndDiagnostics'));

    expect(screen.getByText('screenAwareness.debug.collapse')).toBeInTheDocument();
    expect(screen.getByTestId('screen-debug-inner')).toBeInTheDocument();
  });

  it('collapses debug section after second click', async () => {
    await renderPanel();

    const toggleBtn = screen.getByText('screenAwareness.debug.debugAndDiagnostics');
    fireEvent.click(toggleBtn);
    fireEvent.click(toggleBtn);

    expect(screen.getByText('screenAwareness.debug.expand')).toBeInTheDocument();
    expect(screen.queryByTestId('screen-debug-inner')).not.toBeInTheDocument();
  });

  // ── Policy settings form: FPS input (line 143) ───────────────────────────

  it('renders baseline FPS input with config value (line 143)', async () => {
    await renderPanel({
      status: makeStatus({
        config: {
          baseline_fps: 2,
          use_vision_model: true,
          keep_screenshots: false,
          allowlist: [],
          denylist: [],
          policy_mode: 'all_except_blacklist',
          enabled: true,
        },
      }),
    });

    const fpsInput = screen.getByRole('spinbutton') as HTMLInputElement;
    expect(fpsInput.value).toBe('2');
  });

  it('updates baseline FPS on change', async () => {
    await renderPanel();

    const fpsInput = screen.getByRole('spinbutton');
    fireEvent.change(fpsInput, { target: { value: '5' } });

    expect((fpsInput as HTMLInputElement).value).toBe('5');
  });

  // ── Use vision model checkbox (line 181) ──────────────────────────────────

  it('renders use-vision-model checkbox checked (line 181)', async () => {
    await renderPanel();

    // SettingsCheckbox renders a native <input type="checkbox" id="screen-use-vision-model">.
    // The panel also has a keep-screenshots checkbox, so query by id.
    const checkbox = document.getElementById('screen-use-vision-model') as HTMLInputElement;
    expect(checkbox).toBeInTheDocument();
    // config.use_vision_model=true → checked=true
    expect(checkbox.checked).toBe(true);
    expect(screen.getByText('screenAwareness.debug.useVisionModel')).toBeInTheDocument();
  });

  // ── saveConfig handler (lines 203, 205) ───────────────────────────────────

  it('calls updateScreenIntelligenceSettings when save is clicked (line 203)', async () => {
    const refreshStatus = vi.fn().mockResolvedValue(null);
    mockUseScreenIntelligenceState.mockReturnValue(makeDefaultState({ refreshStatus }));

    const { default: ScreenAwarenessDebugPanel } = await import('../ScreenAwarenessDebugPanel');
    render(<ScreenAwarenessDebugPanel />);

    fireEvent.click(screen.getByText('screenAwareness.debug.saveSettings'));

    await waitFor(() => expect(mockUpdateScreenIntelligenceSettings).toHaveBeenCalled());
    expect(mockUpdateScreenIntelligenceSettings).toHaveBeenCalledWith(
      expect.objectContaining({ enabled: true, use_vision_model: true, keep_screenshots: false })
    );
  });

  it('shows error when saveConfig throws (line 205)', async () => {
    mockUpdateScreenIntelligenceSettings.mockRejectedValue(new Error('permission denied'));

    await renderPanel();
    fireEvent.click(screen.getByText('screenAwareness.debug.saveSettings'));

    await waitFor(() => expect(screen.getByText('permission denied')).toBeInTheDocument());
  });

  it('skips saveConfig when not in tauri env (line 203)', async () => {
    mockIsTauri.mockReturnValue(false);
    await renderPanel();

    fireEvent.click(screen.getByText('screenAwareness.debug.saveSettings'));

    // Should not call update
    expect(mockUpdateScreenIntelligenceSettings).not.toHaveBeenCalled();
  });

  // ── Vision summaries: empty state (line 251) ──────────────────────────────

  it('shows empty state when no vision summaries (line 251)', async () => {
    await renderPanel({ recentVisionSummaries: [] });

    expect(screen.getByText('screenAwareness.debug.noSummaries')).toBeInTheDocument();
  });

  // ── Vision summaries: list rendering (lines 245, 247, 255-256, 261-262) ───

  it('renders vision summary rows with app name and notes (lines 255-256, 261-262)', async () => {
    const summary = makeVisionSummary();
    await renderPanel({ recentVisionSummaries: [summary] });

    // app_name and window_title are text nodes inside a div that also contains the
    // timestamp and bullet separators — use body.textContent to avoid split-node issues.
    expect(document.body.textContent).toContain('TextEdit');
    expect(document.body.textContent).toContain('User is editing a document.');
    expect(document.body.textContent).toContain('Untitled');
  });

  it('renders unknownApp when app_name is null (line 261)', async () => {
    const summary = makeVisionSummary({ app_name: null, window_title: null });
    await renderPanel({ recentVisionSummaries: [summary] });

    expect(document.body.textContent).toContain('screenAwareness.debug.unknownApp');
  });

  it('calls refreshVision when Refresh button is clicked (line 247)', async () => {
    const refreshVision = vi.fn().mockResolvedValue([]);
    await renderPanel({ refreshVision });

    fireEvent.click(screen.getByText('common.refresh'));

    await waitFor(() => expect(refreshVision).toHaveBeenCalledWith(10));
  });

  it('shows refreshing label while vision is loading (line 245)', async () => {
    await renderPanel({ isLoadingVision: true });

    expect(screen.getByText('screenAwareness.debug.refreshing')).toBeInTheDocument();
  });

  // ── Platform unsupported notice and lastError (line 297) ─────────────────

  it('shows macosOnly notice when platform not supported (line 290-292)', async () => {
    const status = makeStatus();
    status.platform_supported = false;
    await renderPanel({ status });

    expect(screen.getByText('screenAwareness.debug.macosOnly')).toBeInTheDocument();
  });

  it('shows lastError status line when lastError is non-null (line 297)', async () => {
    await renderPanel({ lastError: 'screen recording permission denied' });

    expect(screen.getByText('screen recording permission denied')).toBeInTheDocument();
  });

  // ── Session stats section (lines 215-235) ────────────────────────────────

  it('renders session stats with live values', async () => {
    await renderPanel();

    // These keys appear as text nodes inside divs that also hold ': <value>' —
    // use body.textContent to avoid split-text-node matching failures.
    expect(document.body.textContent).toContain('screenAwareness.debug.framesEphemeral');
    expect(document.body.textContent).toContain('screenAwareness.debug.panicStop');
    expect(document.body.textContent).toContain('screenAwareness.debug.vision');
  });

  it('shows defaultPanicHotkey when panic_hotkey is null', async () => {
    const status = makeStatus();
    // At runtime the JSON from core may return null even though the TypeScript type says string.
    // Force-cast to test the nullish-coalescing fallback branch.
    (status.session as unknown as Record<string, unknown>).panic_hotkey = null;
    await renderPanel({ status });

    expect(document.body.textContent).toContain('screenAwareness.debug.defaultPanicHotkey');
  });

  it('shows notAvailable for last_vision_at_ms when null', async () => {
    const status = makeStatus();
    // last_vision_at_ms is number | null in the real type, but inferred as number in the fixture.
    // Force-cast to assign null and test the notAvailable branch.
    (status.session as unknown as Record<string, unknown>).last_vision_at_ms = null;
    await renderPanel({ status });

    expect(document.body.textContent).toContain('screenAwareness.debug.notAvailable');
  });
});
