/**
 * VoiceDebugPanel coverage tests.
 *
 * Target uncovered lines (from diff-cover report):
 * 144,157,178,189,227-228,242-243
 *
 * These cover:
 * - serverStatus rendering: state, last_error panel (lines 155–194)
 * - voiceStatus STT ready/not-ready branch (line 167-170)
 * - serverStatus extra info row (hotkey, mode, transcription_count) (line 175-186)
 * - last_error box (line 189)
 * - Advanced settings section (lines 204–252): min_duration, silence_threshold fields
 * - hasUnsavedChanges → Save button enabled (lines 227-228)
 * - saveSettings success notice (lines 115, 242-243)
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  openhumanGetVoiceServerSettings,
  openhumanUpdateVoiceServerSettings,
  openhumanVoiceServerStatus,
  openhumanVoiceStatus,
  type VoiceServerSettings,
  type VoiceServerStatus,
  type VoiceStatus,
} from '../../../../utils/tauriCommands';
import type { ConfigSnapshot } from '../../../../utils/tauriCommands/config';
import VoiceDebugPanel from '../VoiceDebugPanel';

// Key-passthrough i18n + trivial chrome so we can render the panel standalone.
vi.mock('../../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));
vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: vi.fn(), breadcrumbs: [] }),
}));
vi.mock('../components/SettingsHeader', () => ({ default: () => null }));

vi.mock('../../../../utils/tauriCommands', () => ({
  openhumanGetVoiceServerSettings: vi.fn(),
  openhumanUpdateVoiceServerSettings: vi.fn(),
  openhumanVoiceServerStatus: vi.fn(),
  openhumanVoiceStatus: vi.fn(),
}));

const SETTINGS: VoiceServerSettings = {
  auto_start: false,
  hotkey: 'Fn',
  activation_mode: 'push',
  skip_cleanup: true,
  min_duration_secs: 0.3,
  silence_threshold: 0.002,
  custom_dictionary: [],
  always_on_enabled: false,
};

const SERVER_STATUS: VoiceServerStatus = {
  state: 'idle',
  hotkey: 'Fn',
  activation_mode: 'push',
  transcription_count: 42,
  last_error: null,
};

const VOICE_STATUS: VoiceStatus = {
  stt_available: true,
  tts_available: true,
  stt_model_id: 'ggml-tiny',
  tts_voice_id: 'en_US',
  whisper_binary: null,
  piper_binary: null,
  stt_model_path: null,
  tts_voice_path: null,
  whisper_in_process: true,
  llm_cleanup_enabled: true,
  stt_provider: 'cloud',
  tts_provider: 'cloud',
};

// Always-on listening was relocated to the Desktop Agent panel; the Voice debug
// panel no longer hosts that toggle. Save-flow coverage below drives the
// minimum-recording-seconds field instead.

describe('VoiceDebugPanel — runtime status section (uncovered lines)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(openhumanGetVoiceServerSettings).mockResolvedValue({
      result: { ...SETTINGS },
      logs: [],
    });
    vi.mocked(openhumanUpdateVoiceServerSettings).mockResolvedValue({
      result: {} as unknown as ConfigSnapshot,
      logs: [],
    });
    vi.mocked(openhumanVoiceServerStatus).mockResolvedValue(SERVER_STATUS);
    vi.mocked(openhumanVoiceStatus).mockResolvedValue(VOICE_STATUS);
  });

  // ── Server status state display (line 144, 155-160) ───────────────────────

  it('renders server state from serverStatus (line 155)', async () => {
    // state must be a valid VoiceServerStatus literal; 'recording' is rendered as-is
    vi.mocked(openhumanVoiceServerStatus).mockResolvedValue({
      ...SERVER_STATUS,
      state: 'recording' as const,
    });
    render(<VoiceDebugPanel />);

    await waitFor(() => expect(screen.getByText('recording')).toBeInTheDocument());
  });

  it('shows loading placeholder initially when serverStatus is null (line 157)', async () => {
    // Delay resolution so the panel renders with null serverStatus first
    vi.mocked(openhumanVoiceServerStatus).mockImplementation(
      () => new Promise(resolve => setTimeout(() => resolve(SERVER_STATUS), 200))
    );
    vi.useFakeTimers({ shouldAdvanceTime: true });
    render(<VoiceDebugPanel />);

    // Before any data resolves, "loading" placeholder should appear in server state box
    expect(screen.getByText('common.loading')).toBeInTheDocument();
    vi.useRealTimers();
  });

  // ── STT ready/not-ready branch (line 167-170) ─────────────────────────────

  it('shows ready when stt_available=true (line 167)', async () => {
    render(<VoiceDebugPanel />);

    await waitFor(() => expect(screen.getByText('voice.debug.ready')).toBeInTheDocument());
  });

  it('shows notReady when stt_available=false (line 170)', async () => {
    vi.mocked(openhumanVoiceStatus).mockResolvedValue({ ...VOICE_STATUS, stt_available: false });
    render(<VoiceDebugPanel />);

    await waitFor(() => expect(screen.getByText('voice.debug.notReady')).toBeInTheDocument());
  });

  // ── Extra info row: hotkey, mode, transcription_count (lines 178-184) ─────

  it('renders hotkey and activation_mode and transcription count (lines 178-184)', async () => {
    render(<VoiceDebugPanel />);

    await waitFor(() => expect(screen.getByText(/voice\.debug\.hotkey/)).toBeInTheDocument());
    expect(screen.getByText(/voice\.debug\.mode/)).toBeInTheDocument();
    expect(screen.getByText(/voice\.debug\.transcriptions/)).toBeInTheDocument();
    expect(screen.getByText(/42/)).toBeInTheDocument();
  });

  it('shows notAvailable when hotkey is empty (line 178)', async () => {
    vi.mocked(openhumanVoiceServerStatus).mockResolvedValue({ ...SERVER_STATUS, hotkey: '' });
    render(<VoiceDebugPanel />);

    // voice.debug.notAvailable is a text node inside a div that also contains
    // 'voice.debug.hotkey: ' — use body textContent check to avoid split-node issues.
    await waitFor(() => expect(document.body.textContent).toContain('voice.debug.notAvailable'));
  });

  // ── Last error box (line 189) ─────────────────────────────────────────────

  it('renders last_error panel when serverStatus has last_error (line 189)', async () => {
    vi.mocked(openhumanVoiceServerStatus).mockResolvedValue({
      ...SERVER_STATUS,
      last_error: 'microphone access denied',
    });
    render(<VoiceDebugPanel />);

    await waitFor(() => expect(screen.getByText('microphone access denied')).toBeInTheDocument());
    expect(screen.getByText('voice.debug.serverError')).toBeInTheDocument();
  });

  // ── Refresh button (line 144) ─────────────────────────────────────────────

  it('calls loadData when refresh button is clicked (line 144)', async () => {
    render(<VoiceDebugPanel />);

    await waitFor(() => expect(screen.getByText('common.refresh')).toBeInTheDocument());
    fireEvent.click(screen.getByText('common.refresh'));

    await waitFor(() => expect(vi.mocked(openhumanVoiceServerStatus)).toHaveBeenCalledTimes(2));
  });

  // ── Advanced settings section (lines 204–252) ─────────────────────────────

  it('renders advanced settings section when settings are loaded (line 204)', async () => {
    render(<VoiceDebugPanel />);

    await waitFor(() =>
      expect(screen.getByText('voice.debug.minimumRecordingSeconds')).toBeInTheDocument()
    );
    expect(screen.getByText('voice.debug.silenceThreshold')).toBeInTheDocument();
  });

  // ── Save settings (lines 227-228) ─────────────────────────────────────────

  it('Save button is disabled when no unsaved changes (line 227)', async () => {
    render(<VoiceDebugPanel />);

    await waitFor(() => screen.getByText('common.save'));
    const saveBtn = screen.getByText('common.save').closest('button') as HTMLButtonElement;
    expect(saveBtn.disabled).toBe(true);
  });

  it('enables Save after editing a setting (line 228)', async () => {
    render(<VoiceDebugPanel />);

    const input = await screen.findByLabelText('voice.debug.minimumRecordingSeconds');
    fireEvent.change(input, { target: { value: '5' } });

    const saveBtn = screen.getByText('common.save').closest('button') as HTMLButtonElement;
    expect(saveBtn.disabled).toBe(false);
  });

  // ── saveSettings success notice (lines 115, 242-243) ─────────────────────

  it('shows settingsSaved notice after successful save (line 242-243)', async () => {
    render(<VoiceDebugPanel />);

    const input = await screen.findByLabelText('voice.debug.minimumRecordingSeconds');
    fireEvent.change(input, { target: { value: '5' } });

    fireEvent.click(screen.getByText('common.save'));

    await waitFor(() => expect(screen.getByText('voice.debug.settingsSaved')).toBeInTheDocument());
  });

  it('shows error message when save fails (line 242)', async () => {
    vi.mocked(openhumanUpdateVoiceServerSettings).mockRejectedValue(
      new Error('server refused update')
    );
    render(<VoiceDebugPanel />);

    const input = await screen.findByLabelText('voice.debug.minimumRecordingSeconds');
    fireEvent.change(input, { target: { value: '5' } });

    fireEvent.click(screen.getByText('common.save'));

    await waitFor(() => expect(screen.getByText('server refused update')).toBeInTheDocument());
  });
});
