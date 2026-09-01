import { fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  installPiper,
  piperInstallStatus,
  type VoiceInstallStatus,
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
  installPiper: vi.fn(),
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
  engine: 'piper',
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

/** A registered ElevenLabs entry — the STT engine tests need a provider in the
 *  registry, because the routing dropdown only offers slugs it knows about. */
const ELEVENLABS_PROVIDER = {
  id: '1',
  slug: 'elevenlabs',
  label: 'ElevenLabs',
  endpoint: 'https://api.elevenlabs.io/v1',
  auth_style: 'bearer',
  capability: 'both' as const,
  stt_api_style: 'openai_audio',
  tts_api_style: 'elevenlabs',
  default_stt_model: 'scribe_v1',
  default_tts_voice: 'JBFqnCBsd6RMkjVDRZzb',
  has_api_key: true,
};

/** Build a minimal VoiceSettings with no external providers registered. */
const makeVoiceSettings = (overrides: Partial<VoiceSettings> = {}): VoiceSettings => ({
  voiceProviders: [],
  sttProvider: { kind: 'cloud' },
  ttsProvider: { kind: 'cloud' },
  ...overrides,
});

type RuntimeHarness = {
  settings: VoiceServerSettings;
  voiceStatus: VoiceStatus;
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
        stt_engine: 'backend',
      },
      voiceStatus: {
        stt_available: true,
        tts_available: true,
        stt_model_id: 'ggml-tiny-q5_1.bin',
        tts_voice_id: 'en_US-lessac-medium',
        piper_binary: null,
        tts_voice_path: '/tmp/tts.onnx',
        llm_cleanup_enabled: true,
        stt_engine: 'cloud',
        stt_error: null,
        tts_provider: 'cloud',
      },
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
      if (update.stt_provider) runtime.voiceStatus.stt_engine = update.stt_provider;
      if (update.tts_provider) runtime.voiceStatus.tts_provider = update.tts_provider;
      if (update.stt_model) runtime.voiceStatus.stt_model_id = update.stt_model;
      if (update.tts_voice) runtime.voiceStatus.tts_voice_id = update.tts_voice;
      return {
        stt_provider: runtime.voiceStatus.stt_engine,
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
    // mutate `runtime.piperStatus` to simulate a real install cycle.
    vi.mocked(piperInstallStatus).mockImplementation(async () => ({ ...runtime.piperStatus }));
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

  it('persists always-on listening and syncs the notch indicator', async () => {
    renderWithProviders(<VoicePanel />);

    const toggle = await screen.findByTestId('voice-always-on-toggle');
    expect(toggle).toHaveAttribute('aria-checked', 'false');

    fireEvent.click(toggle);

    await waitFor(() =>
      expect(openhumanUpdateVoiceServerSettings).toHaveBeenCalledWith({ always_on_enabled: true })
    );
    await waitFor(() => expect(syncNotchVisibility).toHaveBeenCalledWith(true));
    expect(toggle).toHaveAttribute('aria-checked', 'true');
  });

  it('restores the always-on toggle when persistence fails', async () => {
    vi.mocked(openhumanUpdateVoiceServerSettings).mockRejectedValueOnce(
      new Error('settings unavailable')
    );
    renderWithProviders(<VoicePanel />);

    const toggle = await screen.findByTestId('voice-always-on-toggle');
    fireEvent.click(toggle);

    await screen.findByText('settings unavailable');
    expect(toggle).toHaveAttribute('aria-checked', 'false');
    expect(syncNotchVisibility).not.toHaveBeenCalled();
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
      voiceProviders: [ELEVENLABS_PROVIDER],
      sttProvider: { kind: 'external', providerSlug: 'elevenlabs', model: 'scribe_v1' },
      ttsProvider: { kind: 'local', engine: 'piper', model: '' },
    });

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const sttSelect = (await screen.findByTestId('stt-provider-select')) as HTMLSelectElement;
    const ttsSelect = (await screen.findByTestId('tts-provider-select')) as HTMLSelectElement;
    // Wait for the seeding effect from loadVoiceSettings.
    await waitFor(() => expect(sttSelect.value).toBe('elevenlabs'));
    expect(ttsSelect.value).toBe('piper');
    // tts_voice_id is seeded to 'en_US-lessac-medium' which is a known preset,
    // so the UI should render the preset select, not the free-text input.
    expect(screen.getByTestId('tts-voice-select')).toBeInTheDocument();
    expect(screen.queryByTestId('tts-voice-input')).not.toBeInTheDocument();
  });

  it('shows the effective hosted STT engine when cloud routing delegates to it', async () => {
    runtime.voiceStatus.stt_engine = 'elevenlabs';
    runtime.voiceSettings = makeVoiceSettings({
      voiceProviders: [ELEVENLABS_PROVIDER],
      sttProvider: { kind: 'cloud' },
    });

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const sttSelect = (await screen.findByTestId('stt-provider-select')) as HTMLSelectElement;
    await waitFor(() => expect(sttSelect.value).toBe('elevenlabs'));
  });

  it('selecting a new STT provider updates local state without immediately calling the RPC', async () => {
    // Seed an external STT provider so the dropdown starts on a non-cloud value.
    runtime.voiceSettings = makeVoiceSettings({
      voiceProviders: [ELEVENLABS_PROVIDER],
      sttProvider: { kind: 'external', providerSlug: 'elevenlabs', model: 'scribe_v1' },
      ttsProvider: { kind: 'cloud' },
    });

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const sttSelect = (await screen.findByTestId('stt-provider-select')) as HTMLSelectElement;
    // Initial value should be elevenlabs (seeded from voiceSettings).
    await waitFor(() => expect(sttSelect.value).toBe('elevenlabs'));

    // Change back to cloud — just updates local state, no RPC yet.
    fireEvent.change(sttSelect, { target: { value: 'cloud' } });
    await waitFor(() => expect(sttSelect.value).toBe('cloud'));

    // No RPC call yet — user must click Save.
    expect(vi.mocked(openhumanVoiceSetProviders)).not.toHaveBeenCalled();
  });

  it('persists STT provider changes through openhumanVoiceSetProviders when Save is clicked', async () => {
    runtime.voiceSettings = makeVoiceSettings({
      voiceProviders: [ELEVENLABS_PROVIDER],
      sttProvider: { kind: 'external', providerSlug: 'elevenlabs', model: 'scribe_v1' },
      ttsProvider: { kind: 'cloud' },
    });

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const sttSelect = (await screen.findByTestId('stt-provider-select')) as HTMLSelectElement;
    await waitFor(() => expect(sttSelect.value).toBe('elevenlabs'));

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
      voiceProviders: [ELEVENLABS_PROVIDER],
      sttProvider: { kind: 'external', providerSlug: 'elevenlabs', model: 'scribe_v1' },
      ttsProvider: { kind: 'cloud' },
    });

    vi.mocked(openhumanVoiceSetProviders).mockRejectedValueOnce(new Error('RPC timeout'));

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    // Wait for the initial load to complete (elevenlabs seeded from voiceSettings).
    const sttSelect = (await screen.findByTestId('stt-provider-select')) as HTMLSelectElement;
    await waitFor(() => expect(sttSelect.value).toBe('elevenlabs'));

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

  it('renders the Piper chip as enabled and clickable (regression #2788)', async () => {
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByTestId('voice-providers-section');
    // The Piper chip must be reachable so users can install and route to the
    // local TTS engine without editing config.toml by hand. The chip is "off"
    // until Piper is the active TTS routing target. There is no STT
    // counterpart: every speech-to-text engine is hosted, so nothing installs.
    const piperChip = await screen.findByTestId('voice-provider-chip-piper');
    expect(piperChip).not.toBeDisabled();
    expect(piperChip).toHaveAttribute('aria-checked', 'false');
    expect(screen.queryByTestId('voice-provider-chip-whisper')).not.toBeInTheDocument();
  });

  it('opens the install modal when the Piper chip is clicked', async () => {
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByTestId('voice-providers-section');
    const piperChip = await screen.findByTestId('voice-provider-chip-piper');
    fireEvent.click(piperChip);

    expect(await screen.findByTestId('voice-provider-key-modal')).toBeInTheDocument();
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

  it('falls back to the voice_status stt_engine when loadVoiceSettings rejects', async () => {
    // Older cores have no voice-provider registry RPC, so the panel seeds the
    // routing dropdown from voice_status instead of rendering an empty picker.
    runtime.voiceStatus.stt_engine = 'cloud';
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

  // ─── Test buttons gate on local-model install completion ────────────────────

  it('disables Test TTS while the selected Piper voice is not installed', async () => {
    runtime.voiceSettings = makeVoiceSettings({
      ttsProvider: { kind: 'local', engine: 'piper', model: '' },
    });
    runtime.piperStatus = makeInstallStatus('piper', { state: 'missing' });
    // Mirror the STT gate: no installed voice and no runtime availability is
    // the real "not installed" case; `piperReady` also keys off `tts_available`.
    runtime.voiceStatus.tts_available = false;

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const ttsSelect = (await screen.findByTestId('tts-provider-select')) as HTMLSelectElement;
    await waitFor(() => expect(ttsSelect.value).toBe('piper'));

    expect(await screen.findByTestId('test-tts-button')).toBeDisabled();
  });

  it('enables Test TTS once the selected Piper voice is installed', async () => {
    runtime.voiceSettings = makeVoiceSettings({
      ttsProvider: { kind: 'local', engine: 'piper', model: '' },
    });
    runtime.piperStatus = makeInstallStatus('piper', { state: 'installed', progress: 100 });

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const ttsSelect = (await screen.findByTestId('tts-provider-select')) as HTMLSelectElement;
    await waitFor(() => expect(ttsSelect.value).toBe('piper'));

    await waitFor(() => expect(screen.getByTestId('test-tts-button')).toBeEnabled());
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

  // ─── Modal: install button (piper in the API-key modal) ────────────────────

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

  // ─── Modal: Enable button for local providers ──────────────────────────────

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

  // ─── Modal: "Test Key" is a dry run (#5896) ───────────────────────────────
  //
  // The regression these guard: the Test handler used to call
  // `handleEnableExternalProvider` before testing, which (a) wrote the key to
  // the keychain and activated the provider before it was known to work, and
  // (b) cleared `pendingKeySlug` — unmounting the modal, so the result alert
  // it then set could never render.

  it('clicking Test Key validates without saving or activating the provider', async () => {
    vi.mocked(testVoiceProvider).mockResolvedValueOnce({ ok: true, detail: 'Key OK' });

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByTestId('voice-providers-section');
    fireEvent.click(screen.getByTestId('voice-provider-chip-elevenlabs'));
    await screen.findByTestId('voice-provider-key-modal');

    const keyInput = screen.getByPlaceholderText(/sk/i);
    fireEvent.change(keyInput, { target: { value: 'sk-candidate-key-1234567890' } });

    fireEvent.click(screen.getByRole('button', { name: /^Test Key$/i }));

    await waitFor(() => expect(vi.mocked(testVoiceProvider)).toHaveBeenCalled());

    // The candidate key travels to the core for validation only.
    expect(vi.mocked(testVoiceProvider)).toHaveBeenCalledWith(
      'stt',
      'elevenlabs',
      true,
      'sk-candidate-key-1234567890'
    );

    // Nothing is persisted and nothing is activated by a test.
    expect(vi.mocked(setVoiceProviderKey)).not.toHaveBeenCalled();
    expect(vi.mocked(saveVoiceSettings)).not.toHaveBeenCalled();
  });

  it('keeps the modal mounted after Test Key so the result is visible', async () => {
    vi.mocked(testVoiceProvider).mockResolvedValueOnce({
      ok: true,
      detail: 'Provider key is valid (12ms)',
    });

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByTestId('voice-providers-section');
    fireEvent.click(screen.getByTestId('voice-provider-chip-elevenlabs'));
    await screen.findByTestId('voice-provider-key-modal');

    fireEvent.change(screen.getByPlaceholderText(/sk/i), {
      target: { value: 'sk-candidate-key-1234567890' },
    });
    fireEvent.click(screen.getByRole('button', { name: /^Test Key$/i }));

    // The alert is the whole point of the button — if the modal has unmounted
    // by the time the result lands, this text never appears.
    expect(await screen.findByText(/Provider key is valid/i)).toBeInTheDocument();
    expect(screen.getByTestId('voice-provider-key-modal')).toBeInTheDocument();
  });

  it('surfaces a failed Test Key without saving the bad key', async () => {
    vi.mocked(testVoiceProvider).mockResolvedValueOnce({
      ok: false,
      detail: 'Key test failed: API returned 401 Unauthorized',
    });

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByTestId('voice-providers-section');
    fireEvent.click(screen.getByTestId('voice-provider-chip-elevenlabs'));
    await screen.findByTestId('voice-provider-key-modal');

    fireEvent.change(screen.getByPlaceholderText(/sk/i), {
      target: { value: 'sk-wrong-key-1234567890' },
    });
    fireEvent.click(screen.getByRole('button', { name: /^Test Key$/i }));

    expect(await screen.findByText(/401 Unauthorized/i)).toBeInTheDocument();
    // The user is left free to correct the key: it was never written.
    expect(vi.mocked(setVoiceProviderKey)).not.toHaveBeenCalled();
    expect(screen.getByTestId('voice-provider-key-modal')).toBeInTheDocument();
  });

  it('renders a thrown Test Key error without saving the key', async () => {
    // A rejected RPC (transport dead, core down) takes the `catch` branch,
    // which is a different path from a resolved `{ ok: false }` verdict.
    vi.mocked(testVoiceProvider).mockRejectedValueOnce(new Error('core unreachable'));

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByTestId('voice-providers-section');
    fireEvent.click(screen.getByTestId('voice-provider-chip-elevenlabs'));
    await screen.findByTestId('voice-provider-key-modal');

    fireEvent.change(screen.getByPlaceholderText(/sk/i), {
      target: { value: 'sk-key-for-a-dead-core' },
    });
    fireEvent.click(screen.getByRole('button', { name: /^Test Key$/i }));

    expect(await screen.findByText(/core unreachable/i)).toBeInTheDocument();
    expect(screen.getByTestId('voice-provider-key-modal')).toBeInTheDocument();
    expect(vi.mocked(setVoiceProviderKey)).not.toHaveBeenCalled();
  });

  it('discards an in-flight Test Key result when the key is edited', async () => {
    // The key field stays editable during a test (it is disabled only while
    // *saving*). Without the request-id guard, key A's verdict lands next to
    // key B and reads as a validation of B.
    let resolveTest: (r: { ok: boolean; detail: string }) => void = () => {};
    vi.mocked(testVoiceProvider).mockReturnValueOnce(
      // Annotated: `VoiceTestResult` is not exported, and a bare `new Promise`
      // would infer `Promise<unknown>` and fail typecheck on the mock.
      new Promise<{ ok: boolean; detail: string }>(resolve => {
        resolveTest = resolve;
      })
    );

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByTestId('voice-providers-section');
    fireEvent.click(screen.getByTestId('voice-provider-chip-elevenlabs'));
    await screen.findByTestId('voice-provider-key-modal');

    const keyInput = screen.getByPlaceholderText(/sk/i);
    fireEvent.change(keyInput, { target: { value: 'sk-key-AAAA-1234567890' } });
    fireEvent.click(screen.getByRole('button', { name: /^Test Key$/i }));

    // User edits to a different key before the verdict for the first arrives.
    fireEvent.change(keyInput, { target: { value: 'sk-key-BBBB-0987654321' } });

    resolveTest({ ok: true, detail: 'STALE VERDICT FOR KEY A' });

    await waitFor(() => expect(screen.getByRole('button', { name: /^Test Key$/i })).toBeEnabled());
    expect(screen.queryByText(/STALE VERDICT FOR KEY A/i)).not.toBeInTheDocument();
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

  // ─── Save routing ─────────────────────────────────────────────────────────

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
