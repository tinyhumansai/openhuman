import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  installPiper,
  installWhisper,
  piperInstallStatus,
  type VoiceInstallStatus,
  whisperInstallStatus,
} from '../../../../services/api/voiceInstallApi';
import {
  clearVoiceProviderKey,
  loadVoiceSettings,
  saveVoiceSettings,
  setVoiceProviderKey,
  testVoiceProvider,
  type VoiceSettings,
} from '../../../../services/api/voiceSettingsApi';
import { renderWithProviders } from '../../../../test/test-utils';
import {
  openhumanGetVoiceServerSettings,
  openhumanUpdateVoiceServerSettings,
  openhumanVoiceSetProviders,
  openhumanVoiceStatus,
  syncNotchVisibility,
  type VoiceServerSettings,
  type VoiceStatus,
} from '../../../../utils/tauriCommands';
import VoicePanel from '../VoicePanel';

vi.mock('../../../../utils/tauriCommands', () => ({
  openhumanGetVoiceServerSettings: vi.fn(),
  openhumanUpdateVoiceServerSettings: vi.fn(),
  openhumanVoiceSetProviders: vi.fn(),
  openhumanVoiceStatus: vi.fn(),
  syncNotchVisibility: vi.fn(),
}));

vi.mock('../../../../services/api/voiceInstallApi', () => ({
  installWhisper: vi.fn(),
  installPiper: vi.fn(),
  whisperInstallStatus: vi.fn(),
  piperInstallStatus: vi.fn(),
}));

vi.mock('../../../../services/api/voiceSettingsApi', async () => {
  const actual = await vi.importActual<typeof import('../../../../services/api/voiceSettingsApi')>(
    '../../../../services/api/voiceSettingsApi'
  );
  return {
    ...actual,
    loadVoiceSettings: vi.fn(),
    saveVoiceSettings: vi.fn(),
    setVoiceProviderKey: vi.fn(),
    clearVoiceProviderKey: vi.fn(),
    testVoiceProvider: vi.fn(),
  };
});

// Mascot voice preview path (issue #1762) goes through the existing
// `synthesizeSpeech` TTS RPC, which is heavy + makes real network calls
// in production. Mocked here so the Preview button click is observable
// without standing up a backend. Other ttsClient exports are
// passed-through so transitive importers (e.g. `useHumanMascot`) still
// resolve their cleanup paths.
vi.mock('../../../../features/human/voice/ttsClient', async () => {
  const actual = await vi.importActual<typeof import('../../../../features/human/voice/ttsClient')>(
    '../../../../features/human/voice/ttsClient'
  );
  return { ...actual, synthesizeSpeech: vi.fn() };
});

const makeInstallStatus = (
  engine: 'whisper' | 'piper',
  overrides: Partial<VoiceInstallStatus> = {}
): VoiceInstallStatus => ({
  engine,
  state: 'missing',
  progress: null,
  downloaded_bytes: null,
  total_bytes: null,
  stage: null,
  error_detail: null,
  ...overrides,
});

/** Build a minimal VoiceSettings with no external providers registered. */
const makeVoiceSettings = (overrides: Partial<VoiceSettings> = {}): VoiceSettings => ({
  voiceProviders: [],
  sttProvider: { kind: 'cloud' },
  ttsProvider: { kind: 'cloud' },
  ...overrides,
});

async function advanceTimersAndFlush(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

type RuntimeHarness = {
  settings: VoiceServerSettings;
  voiceStatus: VoiceStatus;
  whisperStatus: VoiceInstallStatus;
  piperStatus: VoiceInstallStatus;
  voiceSettings: VoiceSettings;
};

describe('VoicePanel', () => {
  let runtime: RuntimeHarness;

  beforeEach(() => {
    vi.clearAllMocks();

    runtime = {
      settings: {
        auto_start: false,
        hotkey: 'Fn',
        activation_mode: 'push',
        skip_cleanup: true,
        min_duration_secs: 0.3,
        silence_threshold: 0.002,
        custom_dictionary: [],
        always_on_enabled: false,
      },
      voiceStatus: {
        stt_available: true,
        tts_available: true,
        stt_model_id: 'ggml-tiny-q5_1.bin',
        tts_voice_id: 'en_US-lessac-medium',
        whisper_binary: null,
        piper_binary: null,
        stt_model_path: '/tmp/stt.bin',
        tts_voice_path: '/tmp/tts.onnx',
        whisper_in_process: true,
        llm_cleanup_enabled: true,
        stt_provider: 'cloud',
        tts_provider: 'cloud',
      },
      whisperStatus: makeInstallStatus('whisper'),
      piperStatus: makeInstallStatus('piper'),
      voiceSettings: makeVoiceSettings(),
    };

    vi.mocked(openhumanGetVoiceServerSettings).mockImplementation(async () => ({
      result: { ...runtime.settings },
      logs: [],
    }));
    vi.mocked(openhumanVoiceStatus).mockImplementation(async () => ({ ...runtime.voiceStatus }));
    // The toggle handler ignores the resolved value (it updates React state
    // optimistically before awaiting), so a minimal cast is enough here.
    vi.mocked(openhumanUpdateVoiceServerSettings).mockResolvedValue({
      result: {},
      logs: [],
    } as never);
    vi.mocked(syncNotchVisibility).mockResolvedValue(undefined);
    vi.mocked(openhumanVoiceSetProviders).mockImplementation(async update => {
      if (update.stt_provider) runtime.voiceStatus.stt_provider = update.stt_provider;
      if (update.tts_provider) runtime.voiceStatus.tts_provider = update.tts_provider;
      if (update.stt_model) runtime.voiceStatus.stt_model_id = update.stt_model;
      if (update.tts_voice) runtime.voiceStatus.tts_voice_id = update.tts_voice;
      return {
        stt_provider: runtime.voiceStatus.stt_provider,
        tts_provider: runtime.voiceStatus.tts_provider,
        stt_model_id: runtime.voiceStatus.stt_model_id,
        tts_voice_id: runtime.voiceStatus.tts_voice_id,
      };
    });

    vi.mocked(loadVoiceSettings).mockImplementation(async () => ({ ...runtime.voiceSettings }));
    vi.mocked(saveVoiceSettings).mockResolvedValue(undefined);
    vi.mocked(setVoiceProviderKey).mockResolvedValue(undefined);
    vi.mocked(clearVoiceProviderKey).mockResolvedValue(undefined);
    vi.mocked(testVoiceProvider).mockResolvedValue({ ok: true, detail: 'OK' });

    // Install-status polls return the current harness snapshot — tests
    // mutate `runtime.whisperStatus` / `runtime.piperStatus` to simulate
    // a real install cycle.
    vi.mocked(whisperInstallStatus).mockImplementation(async () => ({ ...runtime.whisperStatus }));
    vi.mocked(piperInstallStatus).mockImplementation(async () => ({ ...runtime.piperStatus }));
    vi.mocked(installWhisper).mockImplementation(async () => {
      runtime.whisperStatus = makeInstallStatus('whisper', {
        state: 'installed',
        progress: 100,
        stage: 'install complete',
      });
      return { ...runtime.whisperStatus };
    });
    vi.mocked(installPiper).mockImplementation(async () => {
      runtime.piperStatus = makeInstallStatus('piper', {
        state: 'installed',
        progress: 100,
        stage: 'install complete',
      });
      return { ...runtime.piperStatus };
    });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  // ─── Voice Routing Section ──────────────────────────────────────────────

  it('renders the STT and TTS provider dropdowns defaulting to cloud', async () => {
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const sttSelect = (await screen.findByTestId('stt-provider-select')) as HTMLSelectElement;
    const ttsSelect = (await screen.findByTestId('tts-provider-select')) as HTMLSelectElement;
    await waitFor(() => expect(sttSelect.value).toBe('cloud'));
    expect(ttsSelect.value).toBe('cloud');
  });

  it('renders the STT and TTS provider dropdowns seeded from loadVoiceSettings', async () => {
    runtime.voiceSettings = makeVoiceSettings({
      sttProvider: { kind: 'local', engine: 'whisper', model: 'medium' },
      ttsProvider: { kind: 'local', engine: 'piper', model: '' },
    });

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const sttSelect = (await screen.findByTestId('stt-provider-select')) as HTMLSelectElement;
    const ttsSelect = (await screen.findByTestId('tts-provider-select')) as HTMLSelectElement;
    // Wait for the seeding effect from loadVoiceSettings.
    await waitFor(() => expect(sttSelect.value).toBe('whisper'));
    expect(ttsSelect.value).toBe('piper');
    // The Whisper model picker only appears when the STT provider is whisper.
    expect(screen.getByTestId('stt-model-select')).toBeInTheDocument();
    // tts_voice_id is seeded to 'en_US-lessac-medium' which is a known preset,
    // so the UI should render the preset select, not the free-text input.
    expect(screen.getByTestId('tts-voice-select')).toBeInTheDocument();
    expect(screen.queryByTestId('tts-voice-input')).not.toBeInTheDocument();
  });

  it('selecting a new STT provider updates local state without immediately calling the RPC', async () => {
    // Seed whisper so the dropdown option is available and starts selected.
    runtime.voiceSettings = makeVoiceSettings({
      sttProvider: { kind: 'local', engine: 'whisper', model: 'medium' },
      ttsProvider: { kind: 'cloud' },
    });

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const sttSelect = (await screen.findByTestId('stt-provider-select')) as HTMLSelectElement;
    // Initial value should be whisper (seeded from voiceSettings).
    await waitFor(() => expect(sttSelect.value).toBe('whisper'));

    // Change back to cloud — just updates local state, no RPC yet.
    fireEvent.change(sttSelect, { target: { value: 'cloud' } });
    await waitFor(() => expect(sttSelect.value).toBe('cloud'));

    // No RPC call yet — user must click Save.
    expect(vi.mocked(openhumanVoiceSetProviders)).not.toHaveBeenCalled();
  });

  it('persists STT provider changes through openhumanVoiceSetProviders when Save is clicked', async () => {
    runtime.voiceSettings = makeVoiceSettings({
      sttProvider: { kind: 'local', engine: 'whisper', model: 'medium' },
      ttsProvider: { kind: 'cloud' },
    });

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const sttSelect = (await screen.findByTestId('stt-provider-select')) as HTMLSelectElement;
    await waitFor(() => expect(sttSelect.value).toBe('whisper'));

    // Switch back to cloud, then save.
    fireEvent.change(sttSelect, { target: { value: 'cloud' } });
    await waitFor(() => expect(sttSelect.value).toBe('cloud'));

    const saveBtn = screen.getByTestId('save-voice-routing');
    fireEvent.click(saveBtn);

    await waitFor(() =>
      expect(vi.mocked(openhumanVoiceSetProviders)).toHaveBeenCalledWith(
        expect.objectContaining({ stt_provider: 'cloud' })
      )
    );
    expect(await screen.findByText(/Voice providers saved/i)).toBeInTheDocument();
  });

  it('persists TTS provider changes through openhumanVoiceSetProviders when Save is clicked', async () => {
    runtime.voiceSettings = makeVoiceSettings({
      sttProvider: { kind: 'cloud' },
      ttsProvider: { kind: 'local', engine: 'piper', model: '' },
    });

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const ttsSelect = (await screen.findByTestId('tts-provider-select')) as HTMLSelectElement;
    await waitFor(() => expect(ttsSelect.value).toBe('piper'));

    // Switch to cloud, then save.
    fireEvent.change(ttsSelect, { target: { value: 'cloud' } });

    const saveBtn = screen.getByTestId('save-voice-routing');
    fireEvent.click(saveBtn);

    await waitFor(() =>
      expect(vi.mocked(openhumanVoiceSetProviders)).toHaveBeenCalledWith(
        expect.objectContaining({ tts_provider: 'cloud' })
      )
    );
  });

  it('Save button is disabled when no routing changes are pending', async () => {
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const saveBtn = await screen.findByTestId('save-voice-routing');
    // No changes yet — button is disabled.
    expect(saveBtn).toBeDisabled();
  });

  it('shows an error when persistProviders fails', async () => {
    runtime.voiceSettings = makeVoiceSettings({
      sttProvider: { kind: 'local', engine: 'whisper', model: 'medium' },
      ttsProvider: { kind: 'cloud' },
    });

    vi.mocked(openhumanVoiceSetProviders).mockRejectedValueOnce(new Error('RPC timeout'));

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    // Wait for the initial load to complete (whisper seeded from voiceSettings).
    const sttSelect = (await screen.findByTestId('stt-provider-select')) as HTMLSelectElement;
    await waitFor(() => expect(sttSelect.value).toBe('whisper'));

    // Freeze subsequent loadData calls so the error set by persistProviders is
    // not cleared by the automatic reload that fires in saveRouting after
    // persistProviders() returns (without re-throwing).
    vi.mocked(openhumanGetVoiceServerSettings).mockImplementation(
      () => new Promise(() => {}) // hang — prevents error being wiped by reload
    );

    // Change provider and click save to trigger the RPC error.
    fireEvent.change(sttSelect, { target: { value: 'cloud' } });
    const saveBtn = screen.getByTestId('save-voice-routing');
    fireEvent.click(saveBtn);

    await waitFor(() => expect(screen.getByText('RPC timeout')).toBeInTheDocument());
  });

  it('renders a preset select and calls persistProviders when a Piper voice preset is changed', async () => {
    runtime.voiceSettings = makeVoiceSettings({
      sttProvider: { kind: 'cloud' },
      ttsProvider: { kind: 'local', engine: 'piper', model: '' },
    });
    runtime.voiceStatus.tts_voice_id = 'en_US-lessac-medium';

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const ttsSelect = (await screen.findByTestId('tts-provider-select')) as HTMLSelectElement;
    await waitFor(() => expect(ttsSelect.value).toBe('piper'));

    const voiceSelect = (await screen.findByTestId('tts-voice-select')) as HTMLSelectElement;
    fireEvent.change(voiceSelect, { target: { value: 'en_US-ryan-medium' } });

    await waitFor(() =>
      expect(vi.mocked(openhumanVoiceSetProviders)).toHaveBeenCalledWith(
        expect.objectContaining({ tts_voice: 'en_US-ryan-medium' })
      )
    );
  });

  // ─── Provider Chip Rendering ────────────────────────────────────────────

  it('renders the managed cloud chip as always enabled and locked', async () => {
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByTestId('voice-providers-section');
    // The cloud chip aria-label uses the i18n key voice.providers.chip.cloudAria.
    const cloudSwitch = screen.getByRole('switch', {
      name: /OpenHuman managed provider is always enabled/i,
    });
    expect(cloudSwitch).toHaveAttribute('aria-checked', 'true');
    expect(cloudSwitch).toBeDisabled();
  });

  it('renders Whisper and Piper chips as enabled and clickable (regression #2788)', async () => {
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByTestId('voice-providers-section');
    // The Whisper / Piper chips must be reachable so users can install and
    // route to the local STT/TTS engines without editing config.toml by
    // hand. The chip is "off" until the engine is selected as the active
    // STT (whisper) / TTS (piper) routing target.
    const whisperChip = await screen.findByTestId('voice-provider-chip-whisper');
    const piperChip = await screen.findByTestId('voice-provider-chip-piper');
    expect(whisperChip).not.toBeDisabled();
    expect(piperChip).not.toBeDisabled();
    expect(whisperChip).toHaveAttribute('aria-checked', 'false');
    expect(piperChip).toHaveAttribute('aria-checked', 'false');
  });

  it('opens the install modal when the Whisper chip is clicked', async () => {
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByTestId('voice-providers-section');
    const whisperChip = await screen.findByTestId('voice-provider-chip-whisper');
    fireEvent.click(whisperChip);

    // The existing local-provider modal opens with the whisper slug — it
    // contains the install button and Whisper model selector that route
    // through `voice_install_whisper` + `voice_update_provider_settings`.
    expect(await screen.findByTestId('voice-provider-key-modal')).toBeInTheDocument();
  });

  it('opens the install modal when the Piper chip is clicked', async () => {
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByTestId('voice-providers-section');
    const piperChip = await screen.findByTestId('voice-provider-chip-piper');
    fireEvent.click(piperChip);

    expect(await screen.findByTestId('voice-provider-key-modal')).toBeInTheDocument();
  });

  it('renders the Whisper chip as on when STT routing is set to whisper', async () => {
    runtime.voiceSettings = makeVoiceSettings({
      sttProvider: { kind: 'local', engine: 'whisper', model: 'medium' },
      ttsProvider: { kind: 'cloud' },
    });

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByTestId('voice-providers-section');
    const whisperChip = await screen.findByTestId('voice-provider-chip-whisper');
    await waitFor(() => expect(whisperChip).toHaveAttribute('aria-checked', 'true'));
  });

  it('renders the Piper chip as on when TTS routing is set to piper', async () => {
    runtime.voiceSettings = makeVoiceSettings({
      sttProvider: { kind: 'cloud' },
      ttsProvider: { kind: 'local', engine: 'piper', model: '' },
    });

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByTestId('voice-providers-section');
    const piperChip = await screen.findByTestId('voice-provider-chip-piper');
    await waitFor(() => expect(piperChip).toHaveAttribute('aria-checked', 'true'));
  });

  it('renders the ElevenLabs chip as off when no provider is registered', async () => {
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByTestId('voice-providers-section');
    const elevenLabsChip = screen.getByTestId('voice-provider-chip-elevenlabs');
    expect(elevenLabsChip).toHaveAttribute('aria-checked', 'false');
  });

  it('renders the ElevenLabs chip as on when the provider is registered', async () => {
    runtime.voiceSettings = makeVoiceSettings({
      voiceProviders: [
        {
          id: '1',
          slug: 'elevenlabs',
          label: 'ElevenLabs',
          endpoint: 'https://api.elevenlabs.io/v1',
          auth_style: 'bearer',
          capability: 'both',
          stt_api_style: 'openai_audio',
          tts_api_style: 'elevenlabs',
          default_stt_model: 'scribe_v1',
          default_tts_voice: 'JBFqnCBsd6RMkjVDRZzb',
          has_api_key: true,
        },
      ],
      sttProvider: { kind: 'cloud' },
      ttsProvider: { kind: 'cloud' },
    });

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByTestId('voice-providers-section');
    const elevenLabsChip = await screen.findByTestId('voice-provider-chip-elevenlabs');
    await waitFor(() => expect(elevenLabsChip).toHaveAttribute('aria-checked', 'true'));
  });

  it('opens the API key modal when an unregistered external provider chip is clicked', async () => {
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByTestId('voice-providers-section');
    const elevenLabsChip = screen.getByTestId('voice-provider-chip-elevenlabs');
    fireEvent.click(elevenLabsChip);

    expect(await screen.findByTestId('voice-provider-key-modal')).toBeInTheDocument();
  });

  // ─── loadVoiceSettings failure fallback ─────────────────────────────────

  it('falls back to legacy voice_status stt_provider when loadVoiceSettings rejects', async () => {
    runtime.voiceStatus.stt_provider = 'whisper';
    vi.mocked(loadVoiceSettings).mockRejectedValueOnce(new Error('not found'));

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const sttSelect = (await screen.findByTestId('stt-provider-select')) as HTMLSelectElement;
    await waitFor(() => expect(sttSelect.value).toBe('whisper'));
  });

  it('falls back to cloud when loadVoiceSettings rejects and voice_status is cloud', async () => {
    runtime.voiceStatus.stt_provider = 'cloud';
    vi.mocked(loadVoiceSettings).mockRejectedValueOnce(new Error('not found'));

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const sttSelect = (await screen.findByTestId('stt-provider-select')) as HTMLSelectElement;
    await waitFor(() => expect(sttSelect.value).toBe('cloud'));
  });

  // ─── Error / notice display ─────────────────────────────────────────────

  it('shows an error banner when openhumanGetVoiceServerSettings rejects', async () => {
    vi.mocked(openhumanGetVoiceServerSettings).mockRejectedValueOnce(new Error('core offline'));

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await waitFor(() => expect(screen.getByText('core offline')).toBeInTheDocument());
  });

  // Always-on listening toggle moved to DesktopAgentPanel (see its test).

  // ─── STT / TTS Test buttons ────────────────────────────────────────────────

  it('clicking Test STT calls testVoiceProvider and shows success result', async () => {
    vi.mocked(testVoiceProvider).mockResolvedValueOnce({ ok: true, detail: 'STT OK' });

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const testSttBtn = await screen.findByTestId('test-stt-button');
    fireEvent.click(testSttBtn);

    await waitFor(() => expect(vi.mocked(testVoiceProvider)).toHaveBeenCalledWith('stt', 'cloud'));
    expect(await screen.findByText('STT OK')).toBeInTheDocument();
  });

  it('clicking Test STT shows error result when testVoiceProvider rejects', async () => {
    vi.mocked(testVoiceProvider).mockRejectedValueOnce(new Error('STT timeout'));

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const testSttBtn = await screen.findByTestId('test-stt-button');
    fireEvent.click(testSttBtn);

    await waitFor(() => expect(screen.getByText('STT timeout')).toBeInTheDocument());
  });

  it('clicking Test TTS calls testVoiceProvider and shows success result', async () => {
    vi.mocked(testVoiceProvider).mockResolvedValueOnce({ ok: true, detail: 'TTS OK' });

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const testTtsBtn = await screen.findByTestId('test-tts-button');
    fireEvent.click(testTtsBtn);

    await waitFor(() => expect(vi.mocked(testVoiceProvider)).toHaveBeenCalledWith('tts', 'cloud'));
    expect(await screen.findByText('TTS OK')).toBeInTheDocument();
  });

  it('clicking Test TTS shows error result when testVoiceProvider rejects', async () => {
    vi.mocked(testVoiceProvider).mockRejectedValueOnce(new Error('TTS unreachable'));

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const testTtsBtn = await screen.findByTestId('test-tts-button');
    fireEvent.click(testTtsBtn);

    await waitFor(() => expect(screen.getByText('TTS unreachable')).toBeInTheDocument());
  });

  it('Test TTS with elevenlabs provider includes elevenlabs in provider string', async () => {
    // Seed voiceSettings with elevenlabs as a registered external provider
    runtime.voiceSettings = makeVoiceSettings({
      sttProvider: { kind: 'cloud' },
      ttsProvider: { kind: 'external', providerSlug: 'elevenlabs', model: '' },
      voiceProviders: [
        {
          id: 'el-tts-test',
          slug: 'elevenlabs',
          label: 'ElevenLabs',
          endpoint: 'https://api.elevenlabs.io/v1',
          auth_style: 'bearer',
          capability: 'both',
          stt_api_style: 'openai_audio',
          tts_api_style: 'elevenlabs',
          default_stt_model: 'scribe_v1',
          default_tts_voice: 'JBFqnCBsd6RMkjVDRZzb',
          has_api_key: true,
        },
      ],
    });

    vi.mocked(testVoiceProvider).mockResolvedValueOnce({ ok: true, detail: 'EL OK' });

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const ttsSelect = (await screen.findByTestId('tts-provider-select')) as HTMLSelectElement;
    await waitFor(() => expect(ttsSelect.value).toBe('elevenlabs'));

    const testTtsBtn = await screen.findByTestId('test-tts-button');
    fireEvent.click(testTtsBtn);

    await waitFor(() =>
      expect(vi.mocked(testVoiceProvider)).toHaveBeenCalledWith(
        'tts',
        expect.stringContaining('elevenlabs')
      )
    );
  });

  // ─── Whisper model picker in routing section ────────────────────────────────

  it('changing the Whisper model select immediately calls persistProviders', async () => {
    runtime.voiceSettings = makeVoiceSettings({
      sttProvider: { kind: 'local', engine: 'whisper', model: 'medium' },
      ttsProvider: { kind: 'cloud' },
    });

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const sttModelSelect = (await screen.findByTestId('stt-model-select')) as HTMLSelectElement;
    fireEvent.change(sttModelSelect, { target: { value: 'small' } });

    await waitFor(() =>
      expect(vi.mocked(openhumanVoiceSetProviders)).toHaveBeenCalledWith(
        expect.objectContaining({ stt_model: 'small' })
      )
    );
  });

  // ─── TTS voice picker (Piper preset select) ─────────────────────────────────

  it('shows the Piper voice preset select and selecting __custom__ is a no-op', async () => {
    runtime.voiceSettings = makeVoiceSettings({
      sttProvider: { kind: 'cloud' },
      ttsProvider: { kind: 'local', engine: 'piper', model: '' },
    });
    runtime.voiceStatus.tts_voice_id = 'en_US-lessac-medium';

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const ttsVoiceSelect = (await screen.findByTestId('tts-voice-select')) as HTMLSelectElement;
    const beforeCallCount = vi.mocked(openhumanVoiceSetProviders).mock.calls.length;

    // Selecting __custom__ should not trigger persistProviders
    fireEvent.change(ttsVoiceSelect, { target: { value: '__custom__' } });

    // Give async effects time to fire
    await new Promise(r => setTimeout(r, 50));
    expect(vi.mocked(openhumanVoiceSetProviders).mock.calls.length).toBe(beforeCallCount);
  });

  // ─── Modal: install buttons (whisper / piper in the API-key modal) ─────────

  it('clicking Install Whisper inside the modal triggers handleInstallWhisper', async () => {
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByTestId('voice-providers-section');
    const whisperChip = await screen.findByTestId('voice-provider-chip-whisper');
    fireEvent.click(whisperChip);

    await screen.findByTestId('voice-provider-key-modal');

    // The install button label is "Install locally" when engine is not yet installed
    const installBtn = await screen.findByRole('button', { name: /install locally/i });
    fireEvent.click(installBtn);

    await waitFor(() => expect(vi.mocked(installWhisper)).toHaveBeenCalled());
  });

  it('clicking Install Piper inside the modal triggers handleInstallPiper', async () => {
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByTestId('voice-providers-section');
    const piperChip = await screen.findByTestId('voice-provider-chip-piper');
    fireEvent.click(piperChip);

    await screen.findByTestId('voice-provider-key-modal');

    const installBtn = await screen.findByRole('button', { name: /install locally/i });
    fireEvent.click(installBtn);

    await waitFor(() => expect(vi.mocked(installPiper)).toHaveBeenCalled());
  });

  it('polls Whisper install status while the local install is running', async () => {
    vi.mocked(installWhisper).mockImplementationOnce(async () => {
      runtime.whisperStatus = makeInstallStatus('whisper', {
        state: 'installing',
        progress: 0,
        stage: 'downloading',
      });
      return { ...runtime.whisperStatus };
    });

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByTestId('voice-providers-section');
    const whisperChip = await screen.findByTestId('voice-provider-chip-whisper');
    fireEvent.click(whisperChip);
    await screen.findByTestId('voice-provider-key-modal');

    vi.useFakeTimers();
    fireEvent.click(screen.getByRole('button', { name: /install locally/i }));
    await advanceTimersAndFlush(0);

    expect(screen.getByRole('button', { name: /Installing 0%/i })).toBeDisabled();

    runtime.whisperStatus = makeInstallStatus('whisper', {
      state: 'installing',
      progress: 42,
      stage: 'downloading',
    });
    await advanceTimersAndFlush(2_000);

    expect(screen.getByRole('button', { name: /Installing 42%/i })).toBeDisabled();

    runtime.whisperStatus = makeInstallStatus('whisper', {
      state: 'installed',
      progress: 100,
      stage: 'install complete',
    });
    await advanceTimersAndFlush(2_000);

    expect(screen.getByRole('button', { name: /Reinstall locally/i })).toBeInTheDocument();
    expect(screen.getByText(/^Installed$/i)).toBeInTheDocument();
  });

  it('does not start overlapping Whisper install status polls', async () => {
    vi.useFakeTimers();
    runtime.whisperStatus = makeInstallStatus('whisper', {
      state: 'installing',
      progress: 0,
      stage: 'downloading',
    });
    const pendingPoll = deferred<VoiceInstallStatus>();
    let whisperStatusCalls = 0;
    vi.mocked(whisperInstallStatus).mockImplementation(() => {
      whisperStatusCalls += 1;
      if (whisperStatusCalls === 1) return Promise.resolve({ ...runtime.whisperStatus });
      return pendingPoll.promise;
    });

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });
    await advanceTimersAndFlush(0);

    expect(whisperStatusCalls).toBe(2);

    await advanceTimersAndFlush(2_000);
    expect(whisperStatusCalls).toBe(2);

    pendingPoll.resolve(
      makeInstallStatus('whisper', { state: 'installing', progress: 42, stage: 'downloading' })
    );
    await advanceTimersAndFlush(0);
    await advanceTimersAndFlush(2_000);

    expect(whisperStatusCalls).toBe(3);
  });

  // ─── Modal: Enable button for local providers ──────────────────────────────

  it('keeps Enable disabled in the Whisper modal until the model is installed', async () => {
    runtime.voiceStatus.stt_available = false;
    runtime.voiceStatus.stt_model_path = null;
    runtime.voiceStatus.whisper_binary = null;

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByTestId('voice-providers-section');
    const whisperChip = await screen.findByTestId('voice-provider-chip-whisper');
    fireEvent.click(whisperChip);

    await screen.findByTestId('voice-provider-key-modal');

    const enableBtn = screen.getByRole('button', { name: /^Enable$/i });
    expect(enableBtn).toBeDisabled();
    fireEvent.click(enableBtn);

    expect(screen.getByTestId('voice-provider-key-modal')).toBeInTheDocument();
    expect(vi.mocked(openhumanVoiceSetProviders)).not.toHaveBeenCalled();
  });

  it('keeps Enable disabled in the Piper modal until the voice is installed', async () => {
    runtime.voiceStatus.tts_available = false;
    runtime.voiceStatus.tts_voice_path = null;
    runtime.voiceStatus.piper_binary = null;

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByTestId('voice-providers-section');
    const piperChip = await screen.findByTestId('voice-provider-chip-piper');
    fireEvent.click(piperChip);

    await screen.findByTestId('voice-provider-key-modal');

    const enableBtn = screen.getByRole('button', { name: /^Enable$/i });
    expect(enableBtn).toBeDisabled();
    fireEvent.click(enableBtn);

    expect(screen.getByTestId('voice-provider-key-modal')).toBeInTheDocument();
    expect(vi.mocked(openhumanVoiceSetProviders)).not.toHaveBeenCalled();
  });

  it('allows Enable in the Whisper modal when voice_status reports local STT ready', async () => {
    runtime.whisperStatus = makeInstallStatus('whisper');
    runtime.voiceStatus.stt_available = true;
    runtime.voiceStatus.stt_model_path = '/legacy/models/ggml-tiny-q5_1.bin';
    runtime.voiceStatus.whisper_binary = '/usr/local/bin/whisper-cli';

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByTestId('voice-providers-section');
    const whisperChip = await screen.findByTestId('voice-provider-chip-whisper');
    fireEvent.click(whisperChip);

    await screen.findByTestId('voice-provider-key-modal');

    const enableBtn = screen.getByRole('button', { name: /^Enable$/i });
    expect(enableBtn).not.toBeDisabled();
    fireEvent.click(enableBtn);

    await waitFor(() =>
      expect(screen.queryByTestId('voice-provider-key-modal')).not.toBeInTheDocument()
    );
  });

  it('allows Enable in the Piper modal when voice_status reports local TTS ready', async () => {
    runtime.piperStatus = makeInstallStatus('piper');
    runtime.voiceStatus.tts_available = true;
    runtime.voiceStatus.tts_voice_path = '/legacy/voices/en_US-lessac-medium.onnx';
    runtime.voiceStatus.piper_binary = '/usr/local/bin/piper';

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByTestId('voice-providers-section');
    const piperChip = await screen.findByTestId('voice-provider-chip-piper');
    fireEvent.click(piperChip);

    await screen.findByTestId('voice-provider-key-modal');

    const enableBtn = screen.getByRole('button', { name: /^Enable$/i });
    expect(enableBtn).not.toBeDisabled();
    fireEvent.click(enableBtn);

    await waitFor(() =>
      expect(screen.queryByTestId('voice-provider-key-modal')).not.toBeInTheDocument()
    );
  });

  it('clicking Enable inside the Whisper modal calls persistProviders and closes modal', async () => {
    runtime.whisperStatus = makeInstallStatus('whisper', {
      state: 'installed',
      progress: 100,
      stage: 'install complete',
    });

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByTestId('voice-providers-section');
    const whisperChip = await screen.findByTestId('voice-provider-chip-whisper');
    fireEvent.click(whisperChip);

    await screen.findByTestId('voice-provider-key-modal');

    const enableBtn = screen.getByRole('button', { name: /^Enable$/i });
    expect(enableBtn).not.toBeDisabled();
    fireEvent.click(enableBtn);

    // Modal closes
    await waitFor(() =>
      expect(screen.queryByTestId('voice-provider-key-modal')).not.toBeInTheDocument()
    );
  });

  it('clicking Enable inside the Piper modal calls persistProviders and closes modal', async () => {
    runtime.piperStatus = makeInstallStatus('piper', {
      state: 'installed',
      progress: 100,
      stage: 'install complete',
    });

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByTestId('voice-providers-section');
    const piperChip = await screen.findByTestId('voice-provider-chip-piper');
    fireEvent.click(piperChip);

    await screen.findByTestId('voice-provider-key-modal');

    const enableBtn = screen.getByRole('button', { name: /^Enable$/i });
    expect(enableBtn).not.toBeDisabled();
    fireEvent.click(enableBtn);

    await waitFor(() =>
      expect(screen.queryByTestId('voice-provider-key-modal')).not.toBeInTheDocument()
    );
  });

  // ─── Modal: Cancel button ──────────────────────────────────────────────────

  it('clicking Cancel inside the Whisper modal closes it without persisting', async () => {
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByTestId('voice-providers-section');
    const whisperChip = await screen.findByTestId('voice-provider-chip-whisper');
    fireEvent.click(whisperChip);

    await screen.findByTestId('voice-provider-key-modal');
    const cancelBtn = screen.getByRole('button', { name: /^Cancel$/i });
    fireEvent.click(cancelBtn);

    await waitFor(() =>
      expect(screen.queryByTestId('voice-provider-key-modal')).not.toBeInTheDocument()
    );
    // No providers were persisted via RPC
    expect(vi.mocked(openhumanVoiceSetProviders)).not.toHaveBeenCalled();
  });

  // ─── External provider (ElevenLabs) modal API-key flow ────────────────────

  it('opening ElevenLabs modal, entering a key, and clicking Save & Enable calls handlers', async () => {
    vi.mocked(setVoiceProviderKey).mockResolvedValue(undefined);
    vi.mocked(saveVoiceSettings).mockResolvedValue(undefined);

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByTestId('voice-providers-section');
    const elevenLabsChip = screen.getByTestId('voice-provider-chip-elevenlabs');
    fireEvent.click(elevenLabsChip);

    await screen.findByTestId('voice-provider-key-modal');

    // Enter an API key (placeholder is 'sk-…' from i18n)
    const keyInput = screen.getByPlaceholderText(/sk/i);
    fireEvent.change(keyInput, { target: { value: 'sk-test-key-el-1234567890' } });

    const saveBtn = screen.getByRole('button', { name: /save.*enable/i });
    fireEvent.click(saveBtn);

    await waitFor(() => expect(vi.mocked(setVoiceProviderKey)).toHaveBeenCalled());
  });

  it('the ElevenLabs modal Cancel button closes without saving', async () => {
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByTestId('voice-providers-section');
    const elevenLabsChip = screen.getByTestId('voice-provider-chip-elevenlabs');
    fireEvent.click(elevenLabsChip);

    await screen.findByTestId('voice-provider-key-modal');

    const cancelBtn = screen.getByRole('button', { name: /^Cancel$/i });
    fireEvent.click(cancelBtn);

    await waitFor(() =>
      expect(screen.queryByTestId('voice-provider-key-modal')).not.toBeInTheDocument()
    );
    expect(vi.mocked(setVoiceProviderKey)).not.toHaveBeenCalled();
  });

  // ─── Mascot voice link ─────────────────────────────────────────────────────

  it('shows the mascot voice section link when TTS is not Piper', async () => {
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    // Default TTS = cloud, so the mascot voice link section should appear
    await screen.findByTestId('mascot-voice-link');
  });

  it('hides the mascot voice link when TTS provider is piper', async () => {
    runtime.voiceSettings = makeVoiceSettings({
      sttProvider: { kind: 'cloud' },
      ttsProvider: { kind: 'local', engine: 'piper', model: '' },
    });
    runtime.voiceStatus.tts_voice_id = 'en_US-lessac-medium';

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const ttsSelect = (await screen.findByTestId('tts-provider-select')) as HTMLSelectElement;
    await waitFor(() => expect(ttsSelect.value).toBe('piper'));

    expect(screen.queryByTestId('mascot-voice-link')).not.toBeInTheDocument();
  });

  // ─── ElevenLabs voice select in routing section ────────────────────────────

  it('shows the ElevenLabs voice select when TTS provider is elevenlabs', async () => {
    runtime.voiceSettings = makeVoiceSettings({
      sttProvider: { kind: 'cloud' },
      ttsProvider: { kind: 'cloud' },
      voiceProviders: [
        {
          id: 'el-1',
          slug: 'elevenlabs',
          label: 'ElevenLabs',
          endpoint: 'https://api.elevenlabs.io/v1',
          auth_style: 'bearer',
          capability: 'both',
          stt_api_style: 'openai_audio',
          tts_api_style: 'elevenlabs',
          default_stt_model: 'scribe_v1',
          default_tts_voice: 'JBFqnCBsd6RMkjVDRZzb',
          has_api_key: true,
        },
      ],
    });

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const ttsSelect = (await screen.findByTestId('tts-provider-select')) as HTMLSelectElement;
    // Switch to elevenlabs
    fireEvent.change(ttsSelect, { target: { value: 'elevenlabs' } });

    await waitFor(() =>
      expect(screen.queryByTestId('elevenlabs-voice-select')).toBeInTheDocument()
    );
  });

  it('selecting __custom__ in ElevenLabs voice preset is a no-op (does not update state)', async () => {
    runtime.voiceSettings = makeVoiceSettings({
      sttProvider: { kind: 'cloud' },
      ttsProvider: { kind: 'cloud' },
      voiceProviders: [
        {
          id: 'el-2',
          slug: 'elevenlabs',
          label: 'ElevenLabs',
          endpoint: 'https://api.elevenlabs.io/v1',
          auth_style: 'bearer',
          capability: 'both',
          stt_api_style: 'openai_audio',
          tts_api_style: 'elevenlabs',
          default_stt_model: 'scribe_v1',
          default_tts_voice: 'JBFqnCBsd6RMkjVDRZzb',
          has_api_key: true,
        },
      ],
    });

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const ttsSelect = (await screen.findByTestId('tts-provider-select')) as HTMLSelectElement;
    fireEvent.change(ttsSelect, { target: { value: 'elevenlabs' } });

    const elVoiceSelect = (await screen.findByTestId(
      'elevenlabs-voice-select'
    )) as HTMLSelectElement;
    const valueBefore = elVoiceSelect.value;
    fireEvent.change(elVoiceSelect, { target: { value: '__custom__' } });

    // Value should not change to __custom__
    await new Promise(r => setTimeout(r, 50));
    expect(elVoiceSelect.value).toBe(valueBefore);
  });

  // ─── Save routing with installed whisper ──────────────────────────────────

  it('save routing button shows success notice after persisting', async () => {
    runtime.voiceSettings = makeVoiceSettings({
      sttProvider: { kind: 'cloud' },
      ttsProvider: { kind: 'local', engine: 'piper', model: '' },
    });
    runtime.voiceStatus.tts_voice_id = 'en_US-lessac-medium';

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const ttsSelect = (await screen.findByTestId('tts-provider-select')) as HTMLSelectElement;
    await waitFor(() => expect(ttsSelect.value).toBe('piper'));

    // Switch to cloud and save
    fireEvent.change(ttsSelect, { target: { value: 'cloud' } });
    const saveBtn = await screen.findByTestId('save-voice-routing');
    fireEvent.click(saveBtn);

    await waitFor(() => expect(screen.queryByText(/Voice providers saved/i)).toBeInTheDocument());
  });
});
