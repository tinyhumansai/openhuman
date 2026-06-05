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
  transcription_count: 0,
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

describe('VoiceDebugPanel — always-on toggle', () => {
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

  it('toggles always-on and persists it via the update RPC on save', async () => {
    render(<VoiceDebugPanel />);

    const toggle = await screen.findByTestId('voice-always-on-toggle');
    expect(toggle).toHaveAttribute('aria-checked', 'false');

    // Local optimistic flip (creates an unsaved change → enables Save).
    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute('aria-checked', 'true');

    fireEvent.click(screen.getByText('common.save'));

    await waitFor(() =>
      expect(vi.mocked(openhumanUpdateVoiceServerSettings)).toHaveBeenCalledWith(
        expect.objectContaining({ always_on_enabled: true })
      )
    );
  });
});
