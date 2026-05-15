import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  installPiper,
  installWhisper,
  piperInstallStatus,
  type VoiceInstallStatus,
  whisperInstallStatus,
} from '../../../../services/api/voiceInstallApi';
import { renderWithProviders } from '../../../../test/test-utils';
import {
  type CommandResponse,
  type ConfigSnapshot,
  openhumanGetVoiceServerSettings,
  openhumanLocalAiAssetsStatus,
  openhumanUpdateVoiceServerSettings,
  openhumanVoiceServerStart,
  openhumanVoiceServerStatus,
  openhumanVoiceServerStop,
  openhumanVoiceSetProviders,
  openhumanVoiceStatus,
  type VoiceServerSettings,
  type VoiceServerStatus,
  type VoiceStatus,
} from '../../../../utils/tauriCommands';
import VoicePanel from '../VoicePanel';

vi.mock('../../../../utils/tauriCommands', () => ({
  openhumanGetVoiceServerSettings: vi.fn(),
  openhumanLocalAiAssetsStatus: vi.fn(),
  openhumanUpdateVoiceServerSettings: vi.fn(),
  openhumanVoiceServerStart: vi.fn(),
  openhumanVoiceServerStatus: vi.fn(),
  openhumanVoiceServerStop: vi.fn(),
  openhumanVoiceSetProviders: vi.fn(),
  openhumanVoiceStatus: vi.fn(),
}));

vi.mock('../../../../services/api/voiceInstallApi', () => ({
  installWhisper: vi.fn(),
  installPiper: vi.fn(),
  whisperInstallStatus: vi.fn(),
  piperInstallStatus: vi.fn(),
}));

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

type RuntimeHarness = {
  settings: VoiceServerSettings;
  serverStatus: VoiceServerStatus;
  voiceStatus: VoiceStatus;
  sttState: string;
  whisperStatus: VoiceInstallStatus;
  piperStatus: VoiceInstallStatus;
};

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

const makeConfigSnapshot = (): CommandResponse<ConfigSnapshot> => ({
  result: {
    config: {},
    workspace_dir: '/tmp/openhuman-ui',
    config_path: '/tmp/openhuman-ui/config.toml',
  },
  logs: [],
});

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
      },
      serverStatus: {
        state: 'stopped',
        hotkey: 'Fn',
        activation_mode: 'push',
        transcription_count: 0,
        last_error: null,
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
      sttState: 'ready',
      whisperStatus: makeInstallStatus('whisper'),
      piperStatus: makeInstallStatus('piper'),
    };

    vi.mocked(openhumanGetVoiceServerSettings).mockImplementation(async () => ({
      result: { ...runtime.settings },
      logs: [],
    }));
    vi.mocked(openhumanVoiceServerStatus).mockImplementation(async () => ({
      ...runtime.serverStatus,
    }));
    vi.mocked(openhumanVoiceStatus).mockImplementation(async () => ({ ...runtime.voiceStatus }));
    vi.mocked(openhumanLocalAiAssetsStatus).mockImplementation(async () => ({
      result: {
        quantization: 'q4',
        stt: { id: runtime.voiceStatus.stt_model_id, state: runtime.sttState },
      } as never,
      logs: [],
    }));
    vi.mocked(openhumanUpdateVoiceServerSettings).mockImplementation(async update => {
      runtime.settings = { ...runtime.settings, ...update };
      return makeConfigSnapshot();
    });
    vi.mocked(openhumanVoiceServerStart).mockImplementation(async params => {
      runtime.serverStatus = {
        ...runtime.serverStatus,
        state: 'idle',
        hotkey: params?.hotkey ?? runtime.settings.hotkey,
        activation_mode: params?.activation_mode ?? runtime.settings.activation_mode,
      };
      return { ...runtime.serverStatus };
    });
    vi.mocked(openhumanVoiceServerStop).mockImplementation(async () => {
      runtime.serverStatus = { ...runtime.serverStatus, state: 'stopped' };
      return { ...runtime.serverStatus };
    });
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

  it('disables the panel when STT assets are not ready', async () => {
    runtime.sttState = 'missing';
    runtime.voiceStatus.stt_available = false;

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    expect(
      await screen.findByText(/Voice dictation is disabled until the local STT model is downloaded/)
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Start Voice Server' })).toBeDisabled();
  });

  it('starts the voice server with the edited form values', async () => {
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByDisplayValue('Fn');

    fireEvent.change(screen.getByDisplayValue('Fn'), { target: { value: 'F6' } });
    fireEvent.change(screen.getByDisplayValue('Verbatim transcription'), {
      target: { value: 'verbatim' },
    });

    fireEvent.click(screen.getByRole('button', { name: 'Start Voice Server' }));

    await waitFor(() => {
      expect(openhumanUpdateVoiceServerSettings).toHaveBeenCalledWith({
        auto_start: false,
        hotkey: 'F6',
        activation_mode: 'push',
        skip_cleanup: true,
        min_duration_secs: 0.3,
        silence_threshold: 0.002,
        custom_dictionary: [],
      });
    });
    expect(openhumanVoiceServerStart).toHaveBeenCalledWith({
      hotkey: 'F6',
      activation_mode: 'push',
      skip_cleanup: true,
    });
    expect(await screen.findByText('Voice server started.')).toBeInTheDocument();
  });

  it('restarts the running server when saving updated settings', async () => {
    runtime.serverStatus.state = 'idle';

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByDisplayValue('Fn');

    fireEvent.click(
      screen.getByLabelText('Start voice server automatically with the core') as HTMLInputElement
    );
    fireEvent.click(screen.getByRole('button', { name: 'Save Voice Settings' }));

    await waitFor(() => {
      expect(openhumanUpdateVoiceServerSettings).toHaveBeenCalledWith({
        auto_start: true,
        hotkey: 'Fn',
        activation_mode: 'push',
        skip_cleanup: true,
        min_duration_secs: 0.3,
        silence_threshold: 0.002,
        custom_dictionary: [],
      });
    });
    expect(openhumanVoiceServerStop).toHaveBeenCalled();
    expect(openhumanVoiceServerStart).toHaveBeenCalledWith({
      hotkey: 'Fn',
      activation_mode: 'push',
      skip_cleanup: true,
    });
    expect(
      await screen.findByText('Voice server restarted with the new settings.')
    ).toBeInTheDocument();
  });

  it('renders the STT and TTS provider dropdowns with seeded values', async () => {
    runtime.voiceStatus.stt_provider = 'whisper';
    runtime.voiceStatus.tts_provider = 'piper';

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const sttSelect = (await screen.findByTestId('stt-provider-select')) as HTMLSelectElement;
    const ttsSelect = (await screen.findByTestId('tts-provider-select')) as HTMLSelectElement;
    // Initial load runs an extra interval tick; wait for the seeding effect.
    await waitFor(() => expect(sttSelect.value).toBe('whisper'));
    expect(ttsSelect.value).toBe('piper');
    // The Whisper model picker only appears when the STT provider is local.
    expect(screen.getByTestId('stt-model-select')).toBeInTheDocument();
    // tts_voice_id is seeded to 'en_US-lessac-medium' which is a known preset,
    // so the UI should render the preset select, not the free-text input.
    expect(screen.getByTestId('tts-voice-select')).toBeInTheDocument();
    expect(screen.queryByTestId('tts-voice-input')).not.toBeInTheDocument();
  });

  it('persists STT provider changes through openhumanVoiceSetProviders', async () => {
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const sttSelect = (await screen.findByTestId('stt-provider-select')) as HTMLSelectElement;
    fireEvent.change(sttSelect, { target: { value: 'whisper' } });

    await waitFor(() =>
      expect(vi.mocked(openhumanVoiceSetProviders)).toHaveBeenCalledWith(
        expect.objectContaining({ stt_provider: 'whisper' })
      )
    );
    // Saved notice should surface for the user.
    expect(await screen.findByText(/Voice providers saved/i)).toBeInTheDocument();
  });

  it('persists TTS provider changes through openhumanVoiceSetProviders', async () => {
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const ttsSelect = (await screen.findByTestId('tts-provider-select')) as HTMLSelectElement;
    fireEvent.change(ttsSelect, { target: { value: 'piper' } });

    await waitFor(() =>
      expect(vi.mocked(openhumanVoiceSetProviders)).toHaveBeenCalledWith(
        expect.objectContaining({ tts_provider: 'piper' })
      )
    );
  });

  it('renders the Install Whisper button when the engine is missing', async () => {
    runtime.whisperStatus = makeInstallStatus('whisper'); // explicit missing
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const button = await screen.findByTestId('install-whisper-button');
    expect(button).toHaveTextContent('Install locally');
    expect(screen.getByTestId('whisper-install-state')).toHaveTextContent('Not installed');
  });

  it('disables the Local Whisper STT option when the engine is missing', async () => {
    runtime.whisperStatus = makeInstallStatus('whisper');
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const sttSelect = (await screen.findByTestId('stt-provider-select')) as HTMLSelectElement;
    const whisperOption = sttSelect.querySelector(
      'option[value="whisper"]'
    ) as HTMLOptionElement | null;
    expect(whisperOption).not.toBeNull();
    expect(whisperOption!.disabled).toBe(true);
    expect(whisperOption!.textContent).toMatch(/install required/i);
  });

  it('shows a Reinstall label once Whisper is installed', async () => {
    runtime.whisperStatus = makeInstallStatus('whisper', { state: 'installed', progress: 100 });
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const button = await screen.findByTestId('install-whisper-button');
    await waitFor(() => expect(button).toHaveTextContent(/Reinstall locally/i));
    expect(screen.getByTestId('whisper-install-state')).toHaveTextContent('Installed');
  });

  it('triggers installWhisper when the user clicks Install', async () => {
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const button = await screen.findByTestId('install-whisper-button');
    fireEvent.click(button);

    await waitFor(() => expect(vi.mocked(installWhisper)).toHaveBeenCalledTimes(1));
    // First-time install must NOT force re-download.
    expect(vi.mocked(installWhisper)).toHaveBeenCalledWith(
      expect.objectContaining({ force: false })
    );
  });

  it('forces re-download when Reinstall is clicked on an installed engine', async () => {
    runtime.whisperStatus = makeInstallStatus('whisper', { state: 'installed', progress: 100 });
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const button = await screen.findByTestId('install-whisper-button');
    await waitFor(() => expect(button).toHaveTextContent(/Reinstall locally/i));
    fireEvent.click(button);

    await waitFor(() => expect(vi.mocked(installWhisper)).toHaveBeenCalledTimes(1));
    expect(vi.mocked(installWhisper)).toHaveBeenCalledWith(
      expect.objectContaining({ force: true })
    );
  });

  it('renders the Install Piper button when the engine is missing', async () => {
    runtime.piperStatus = makeInstallStatus('piper');
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const button = await screen.findByTestId('install-piper-button');
    expect(button).toHaveTextContent('Install locally');
    expect(screen.getByTestId('piper-install-state')).toHaveTextContent('Not installed');
  });

  it('disables the Local Piper TTS option when the engine is missing', async () => {
    runtime.piperStatus = makeInstallStatus('piper');
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const ttsSelect = (await screen.findByTestId('tts-provider-select')) as HTMLSelectElement;
    const piperOption = ttsSelect.querySelector(
      'option[value="piper"]'
    ) as HTMLOptionElement | null;
    expect(piperOption).not.toBeNull();
    expect(piperOption!.disabled).toBe(true);
    expect(piperOption!.textContent).toMatch(/install required/i);
  });

  it('triggers installPiper when the user clicks Install', async () => {
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const button = await screen.findByTestId('install-piper-button');
    fireEvent.click(button);

    await waitFor(() => expect(vi.mocked(installPiper)).toHaveBeenCalledTimes(1));
    expect(vi.mocked(installPiper)).toHaveBeenCalledWith(expect.objectContaining({ force: false }));
  });

  it('shows the in-flight installing label and percentage', async () => {
    runtime.whisperStatus = makeInstallStatus('whisper', {
      state: 'installing',
      progress: 42,
      stage: 'downloading model',
    });
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const stateSpan = await screen.findByTestId('whisper-install-state');
    await waitFor(() => expect(stateSpan).toHaveTextContent(/downloading model/i));
  });

  it('surfaces an error_detail in the install state line', async () => {
    runtime.piperStatus = makeInstallStatus('piper', {
      state: 'error',
      error_detail: 'network unreachable',
    });
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await waitFor(() =>
      expect(screen.getByTestId('piper-install-state')).toHaveTextContent('network unreachable')
    );
    // Button label flips into the retry messaging.
    expect(screen.getByTestId('install-piper-button')).toHaveTextContent(/Retry locally/i);
  });

  it('shows an error notice when installWhisper rejects', async () => {
    // Freeze subsequent loadData calls so the error isn't cleared by the
    // automatic reload that fires in the finally block.
    vi.mocked(installWhisper).mockRejectedValueOnce(new Error('disk full'));
    vi.mocked(openhumanGetVoiceServerSettings).mockImplementation(
      () => new Promise(() => {}) // hang — prevents error being wiped by reload
    );
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    // Wait for the initial load to complete (which uses the pre-hang impl)
    await screen.findByTestId('install-whisper-button');
    // Now freeze subsequent calls and click
    const button = screen.getByTestId('install-whisper-button');
    fireEvent.click(button);

    await waitFor(() => expect(screen.queryByText('disk full')).toBeInTheDocument());
  });

  it('shows an error notice when installPiper rejects', async () => {
    vi.mocked(installPiper).mockRejectedValueOnce(new Error('no space left'));
    vi.mocked(openhumanGetVoiceServerSettings).mockImplementation(
      () => new Promise(() => {}) // hang — prevents error being wiped by reload
    );
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByTestId('install-piper-button');
    const button = screen.getByTestId('install-piper-button');
    fireEvent.click(button);

    await waitFor(() => expect(screen.queryByText('no space left')).toBeInTheDocument());
  });

  it('shows an error when persistProviders fails', async () => {
    vi.mocked(openhumanVoiceSetProviders).mockRejectedValueOnce(new Error('RPC timeout'));
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const sttSelect = (await screen.findByTestId('stt-provider-select')) as HTMLSelectElement;
    fireEvent.change(sttSelect, { target: { value: 'whisper' } });

    await waitFor(() => expect(screen.getByText('RPC timeout')).toBeInTheDocument());
  });

  it('shows a Piper installing label with percentage', async () => {
    runtime.piperStatus = makeInstallStatus('piper', {
      state: 'installing',
      progress: 55,
      stage: 'downloading voice',
    });
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const stateSpan = await screen.findByTestId('piper-install-state');
    await waitFor(() => expect(stateSpan).toHaveTextContent(/downloading voice/i));
  });

  it('renders a preset select and auto-installs when a Piper voice preset is changed', async () => {
    runtime.voiceStatus.tts_provider = 'piper';
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

  // Issue #1762 — Mascot Voice picker tests. Nested inside the outer
  // describe so the runtime mocks (openhumanVoiceStatus seeded with
  // tts_provider='cloud', etc.) are inherited. The section only renders
  // when the cloud (ElevenLabs proxy) TTS provider is active; local
  // Piper has its own picker above. The slice handles validation +
  // persistence; these tests pin the UI surface that drives it.
  describe('Mascot Voice picker (#1762)', () => {
    beforeEach(async () => {
      // Stub a fast successful TTS so the Preview happy-path doesn't
      // wedge on a hanging promise. Individual tests override per case.
      const { synthesizeSpeech } = await import('../../../../features/human/voice/ttsClient');
      vi.mocked(synthesizeSpeech).mockResolvedValue({
        audio_base64: 'AAAA',
        audio_mime: 'audio/mpeg',
        visemes: [],
      });
    });

    it('omits the Mascot Voice section when TTS provider is piper', async () => {
      // Bias the voice status snapshot so the panel mounts in piper mode
      // — the section should be hidden in that case (local voices use the
      // Piper picker above, not the ElevenLabs one).
      const { default: VoicePanel } = await import('../VoicePanel');
      vi.mocked(openhumanVoiceStatus).mockResolvedValueOnce({
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
        tts_provider: 'piper',
      });
      renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });
      // Give the panel a tick to read provider state before asserting.
      await waitFor(() => {
        expect(screen.queryByTestId('mascot-voice-section')).toBeNull();
      });
    });

    it('renders the Mascot Voice section under cloud TTS with the default voice selected', async () => {
      const { default: VoicePanel } = await import('../VoicePanel');
      renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });
      // Wait for the panel to load and seed ttsProvider so the section
      // appears. The dropdown is the gate-keeper of the section.
      const section = await screen.findByTestId('mascot-voice-section');
      // DEBUG: full DOM if section appears empty
      // eslint-disable-next-line no-console
      console.log('SECTION HTML:', section.outerHTML.slice(0, 2000));
      const select = (await screen.findByTestId('mascot-voice-select')) as HTMLSelectElement;
      // With no override stored, the picker reflects the build-time default.
      expect(select.value).toBe('ljX1ZrXuDIIRVcmiVSyR');
      const reset = await screen.findByTestId('mascot-voice-reset');
      expect(reset).toBeDisabled();
    });

    it('switching to a preset voice updates the picker + enables Reset', async () => {
      const { default: VoicePanel } = await import('../VoicePanel');
      renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });
      const select = (await screen.findByTestId('mascot-voice-select')) as HTMLSelectElement;
      fireEvent.change(select, { target: { value: '21m00Tcm4TlvDq8ikWAM' } });
      await waitFor(() => expect(select.value).toBe('21m00Tcm4TlvDq8ikWAM'));
      expect(screen.getByTestId('mascot-voice-reset')).not.toBeDisabled();
      expect(screen.getByTestId('mascot-voice-current').textContent).toContain(
        '21m00Tcm4TlvDq8ikWAM'
      );
    });

    it('selecting "Other (paste voice id)" reveals the custom paste input', async () => {
      const { default: VoicePanel } = await import('../VoicePanel');
      renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });
      const select = (await screen.findByTestId('mascot-voice-select')) as HTMLSelectElement;
      fireEvent.change(select, { target: { value: '__custom__' } });
      expect(await screen.findByTestId('mascot-voice-input')).toBeInTheDocument();
    });

    it('Save commits the pasted voice id and surfaces it as current', async () => {
      const { default: VoicePanel } = await import('../VoicePanel');
      renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });
      const select = (await screen.findByTestId('mascot-voice-select')) as HTMLSelectElement;
      fireEvent.change(select, { target: { value: '__custom__' } });
      const input = (await screen.findByTestId('mascot-voice-input')) as HTMLInputElement;
      fireEvent.change(input, { target: { value: 'custom-paste-id' } });
      fireEvent.click(screen.getByTestId('mascot-voice-save-paste'));
      await waitFor(() =>
        expect(screen.getByTestId('mascot-voice-current').textContent).toContain('custom-paste-id')
      );
    });

    it('Preview calls synthesizeSpeech with the effective voice id', async () => {
      const { synthesizeSpeech } = await import('../../../../features/human/voice/ttsClient');
      const { default: VoicePanel } = await import('../VoicePanel');
      renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });
      const select = (await screen.findByTestId('mascot-voice-select')) as HTMLSelectElement;
      fireEvent.change(select, { target: { value: 'pNInz6obpgDQGcFmaJgB' } }); // Adam
      fireEvent.click(screen.getByTestId('mascot-voice-preview'));
      await waitFor(() =>
        expect(vi.mocked(synthesizeSpeech)).toHaveBeenCalledWith(
          expect.any(String),
          expect.objectContaining({ voiceId: 'pNInz6obpgDQGcFmaJgB' })
        )
      );
    });

    it('Preview failure surfaces a recoverable error banner without dropping the selection', async () => {
      const { synthesizeSpeech } = await import('../../../../features/human/voice/ttsClient');
      vi.mocked(synthesizeSpeech).mockRejectedValueOnce(new Error('Backend unreachable'));
      const { default: VoicePanel } = await import('../VoicePanel');
      renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });
      const select = (await screen.findByTestId('mascot-voice-select')) as HTMLSelectElement;
      fireEvent.change(select, { target: { value: 'EXAVITQu4vr4xnSDxMaL' } }); // Bella
      fireEvent.click(screen.getByTestId('mascot-voice-preview'));
      const banner = await screen.findByTestId('mascot-voice-preview-error');
      expect(banner.textContent).toContain('Backend unreachable');
      expect(banner.textContent).toContain('fall back');
      // Selection survived the failed preview — fallback only applies to
      // the next reply if the chosen voice itself proves unavailable.
      expect((screen.getByTestId('mascot-voice-select') as HTMLSelectElement).value).toBe(
        'EXAVITQu4vr4xnSDxMaL'
      );
    });

    it('Reset clears the override and reflects the build-time default in the picker', async () => {
      const { default: VoicePanel } = await import('../VoicePanel');
      renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });
      const select = (await screen.findByTestId('mascot-voice-select')) as HTMLSelectElement;
      fireEvent.change(select, { target: { value: 'pNInz6obpgDQGcFmaJgB' } });
      await waitFor(() => expect(select.value).toBe('pNInz6obpgDQGcFmaJgB'));
      fireEvent.click(screen.getByTestId('mascot-voice-reset'));
      await waitFor(() => expect(select.value).toBe('ljX1ZrXuDIIRVcmiVSyR'));
      expect(screen.getByTestId('mascot-voice-reset')).toBeDisabled();
    });
  });
});
