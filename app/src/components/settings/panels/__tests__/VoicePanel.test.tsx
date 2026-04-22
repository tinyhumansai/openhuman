import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import {
  type CommandResponse,
  type ConfigSnapshot,
  type LocalAiDownloadsProgress,
  openhumanGetVoiceServerSettings,
  openhumanLocalAiAssetsStatus,
  openhumanLocalAiDownloadAsset,
  openhumanLocalAiDownloadsProgress,
  openhumanUpdateVoiceServerSettings,
  openhumanVoiceServerStart,
  openhumanVoiceServerStatus,
  openhumanVoiceServerStop,
  openhumanVoiceStatus,
  type VoiceServerSettings,
  type VoiceServerStatus,
  type VoiceStatus,
} from '../../../../utils/tauriCommands';
import VoicePanel from '../VoicePanel';

vi.mock('../../../../utils/tauriCommands', () => ({
  openhumanGetVoiceServerSettings: vi.fn(),
  openhumanLocalAiAssetsStatus: vi.fn(),
  openhumanLocalAiDownloadAsset: vi.fn(),
  openhumanLocalAiDownloadsProgress: vi.fn(),
  openhumanUpdateVoiceServerSettings: vi.fn(),
  openhumanVoiceServerStart: vi.fn(),
  openhumanVoiceServerStatus: vi.fn(),
  openhumanVoiceServerStop: vi.fn(),
  openhumanVoiceStatus: vi.fn(),
}));

const emptyDownloadsProgress = (): CommandResponse<LocalAiDownloadsProgress> => {
  const blank = {
    id: '',
    provider: 'local',
    state: 'idle',
    progress: null,
    downloaded_bytes: null,
    total_bytes: null,
    speed_bps: null,
    eta_seconds: null,
    warning: null,
    path: null,
  };
  return {
    result: {
      state: 'idle',
      progress: null,
      downloaded_bytes: null,
      total_bytes: null,
      speed_bps: null,
      eta_seconds: null,
      warning: null,
      chat: { ...blank },
      vision: { ...blank },
      embedding: { ...blank },
      stt: { ...blank },
      tts: { ...blank },
    },
    logs: [],
  };
};

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
      },
      sttState: 'ready',
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
    vi.mocked(openhumanLocalAiDownloadsProgress).mockImplementation(async () =>
      emptyDownloadsProgress()
    );
    vi.mocked(openhumanLocalAiDownloadAsset).mockImplementation(async () => ({
      result: {
        quantization: 'q4',
        stt: { id: 'ggml-tiny-q5_1.bin', state: 'downloading' },
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
  });

  it('shows an inline Download STT model CTA when the STT asset is missing', async () => {
    runtime.sttState = 'missing';
    runtime.voiceStatus.stt_available = false;

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    expect(await screen.findByText('Voice Dictation')).toBeInTheDocument();
    expect(await screen.findByTestId('voice-stt-setup-idle')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Download STT model' })).toBeInTheDocument();
    // Old redirect CTA is gone — the whole point of issue #632.
    expect(screen.queryByRole('button', { name: 'Open Local AI Model' })).toBeNull();
    expect(screen.getByRole('button', { name: 'Start Voice Server' })).toBeDisabled();
  });

  it('triggers an in-place STT download and shows progress without redirecting', async () => {
    runtime.sttState = 'missing';
    runtime.voiceStatus.stt_available = false;

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const cta = await screen.findByRole('button', { name: 'Download STT model' });
    fireEvent.click(cta);

    await waitFor(() => {
      expect(openhumanLocalAiDownloadAsset).toHaveBeenCalledWith('stt');
    });

    // Core starts reporting an active download on the next poll.
    vi.mocked(openhumanLocalAiDownloadsProgress).mockImplementation(async () => {
      const base = emptyDownloadsProgress();
      base.result.state = 'downloading';
      base.result.progress = 0.42;
      base.result.downloaded_bytes = 42_000_000;
      base.result.total_bytes = 100_000_000;
      base.result.stt = {
        id: 'ggml-tiny-q5_1.bin',
        provider: 'local',
        state: 'downloading',
        progress: 0.42,
        downloaded_bytes: 42_000_000,
        total_bytes: 100_000_000,
        speed_bps: 1_500_000,
        eta_seconds: 40,
        warning: null,
        path: null,
      };
      return base;
    });

    expect(await screen.findByTestId('voice-stt-setup-progress')).toBeInTheDocument();
  });

  it('surfaces a retry button when the STT download fails and retries on click', async () => {
    runtime.sttState = 'missing';
    runtime.voiceStatus.stt_available = false;
    vi.mocked(openhumanLocalAiDownloadAsset).mockRejectedValueOnce(new Error('network dropped'));

    renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    fireEvent.click(await screen.findByRole('button', { name: 'Download STT model' }));

    const errorBlock = await screen.findByTestId('voice-stt-setup-error');
    expect(errorBlock).toHaveTextContent('network dropped');
    const retry = screen.getByRole('button', { name: 'Retry download' });

    vi.mocked(openhumanLocalAiDownloadAsset).mockResolvedValueOnce({
      result: {
        quantization: 'q4',
        stt: { id: 'ggml-tiny-q5_1.bin', state: 'downloading' },
      } as never,
      logs: [],
    });
    fireEvent.click(retry);

    await waitFor(() => {
      expect(openhumanLocalAiDownloadAsset).toHaveBeenCalledTimes(2);
    });
    await waitFor(() => {
      expect(screen.queryByTestId('voice-stt-setup-error')).toBeNull();
    });
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
});
