import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import {
  openhumanGetVoiceServerSettings,
  openhumanLocalAiAssetsStatus,
  openhumanUpdateVoiceServerSettings,
  openhumanVoiceServerStart,
  openhumanVoiceServerStatus,
  openhumanVoiceServerStop,
  openhumanVoiceStatus,
  type CommandResponse,
  type ConfigSnapshot,
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
  openhumanVoiceStatus: vi.fn(),
}));

type RuntimeHarness = {
  settings: VoiceServerSettings;
  serverStatus: VoiceServerStatus;
  voiceStatus: VoiceStatus;
  sttState: string;
};

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
        skip_cleanup: false,
        min_duration_secs: 0.3,
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
      },
      sttState: 'ready',
    };

    vi.mocked(openhumanGetVoiceServerSettings).mockImplementation(async () => ({
      result: { ...runtime.settings },
      logs: [],
    }));
    vi.mocked(openhumanVoiceServerStatus).mockImplementation(async () => ({
      result: { ...runtime.serverStatus },
      logs: [],
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
      return { result: { ...runtime.serverStatus }, logs: [] };
    });
    vi.mocked(openhumanVoiceServerStop).mockImplementation(async () => {
      runtime.serverStatus = { ...runtime.serverStatus, state: 'stopped' };
      return { result: { ...runtime.serverStatus }, logs: [] };
    });
  });

  it('disables the panel when STT assets are not ready', async () => {
    runtime.sttState = 'missing';
    runtime.voiceStatus.stt_available = false;

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    expect(await screen.findByText('Voice Dictation')).toBeInTheDocument();
    expect(
      screen.getByText(/Voice dictation is disabled until the local STT model is downloaded/)
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Start Voice Server' })).toBeDisabled();
  });

  it('starts the voice server with the edited form values', async () => {
    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    await screen.findByDisplayValue('Fn');

    fireEvent.change(screen.getByDisplayValue('Fn'), { target: { value: 'F6' } });
    fireEvent.change(screen.getByDisplayValue('Natural cleanup'), {
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
        skip_cleanup: false,
        min_duration_secs: 0.3,
      });
    });
    expect(openhumanVoiceServerStop).toHaveBeenCalled();
    expect(openhumanVoiceServerStart).toHaveBeenCalledWith({
      hotkey: 'Fn',
      activation_mode: 'push',
      skip_cleanup: false,
    });
    expect(await screen.findByText('Voice server restarted with the new settings.')).toBeInTheDocument();
  });
});
