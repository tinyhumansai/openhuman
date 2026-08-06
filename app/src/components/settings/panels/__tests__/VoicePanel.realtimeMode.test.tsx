/**
 * Unit test for VoicePanel's realtime voice-mode toggle (#5399). The section is
 * gated behind the `VOICE_MODE_FLAG_ENABLED` build flag (global setup ships it
 * OFF), so it is flipped ON here. Toggling the switch dispatches
 * `setVoiceMode('realtime')` against the mascot slice. Mount-time voice APIs are
 * stubbed so the panel renders without a backend.
 */
import { act, fireEvent, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import VoicePanel from '../VoicePanel';

// Flip the realtime gate ON for this file; keep every other config export real.
vi.mock('../../../../utils/config', async () => {
  const actual = await vi.importActual<typeof import('../../../../utils/config')>(
    '../../../../utils/config'
  );
  return { ...actual, VOICE_MODE_FLAG_ENABLED: true };
});

vi.mock('../../../../utils/tauriCommands', () => ({
  openhumanGetVoiceServerSettings: vi.fn(async () => ({ result: {}, logs: [] })),
  openhumanUpdateVoiceServerSettings: vi.fn(async () => ({ result: {}, logs: [] })),
  openhumanVoiceSetProviders: vi.fn(async () => ({})),
  openhumanVoiceStatus: vi.fn(async () => ({ stt_provider: 'cloud', tts_provider: 'cloud' })),
  syncNotchVisibility: vi.fn(async () => undefined),
}));

vi.mock('../../../../services/api/voiceInstallApi', () => ({
  installWhisper: vi.fn(),
  installPiper: vi.fn(),
  whisperInstallStatus: vi.fn(async () => ({ engine: 'whisper', state: 'missing' })),
  piperInstallStatus: vi.fn(async () => ({ engine: 'piper', state: 'missing' })),
}));

vi.mock('../../../../services/api/voiceSettingsApi', async () => {
  const actual = await vi.importActual<typeof import('../../../../services/api/voiceSettingsApi')>(
    '../../../../services/api/voiceSettingsApi'
  );
  return {
    ...actual,
    loadVoiceSettings: vi.fn(async () => ({
      voiceProviders: [],
      sttProvider: { kind: 'cloud' },
      ttsProvider: { kind: 'cloud' },
    })),
    saveVoiceSettings: vi.fn(async () => undefined),
    setVoiceProviderKey: vi.fn(async () => undefined),
    clearVoiceProviderKey: vi.fn(async () => undefined),
    testVoiceProvider: vi.fn(async () => ({ ok: true, detail: 'OK' })),
  };
});

vi.mock('../../../../features/human/voice/ttsClient', async () => {
  const actual = await vi.importActual<typeof import('../../../../features/human/voice/ttsClient')>(
    '../../../../features/human/voice/ttsClient'
  );
  return { ...actual, synthesizeSpeech: vi.fn() };
});

describe('VoicePanel — realtime voice-mode toggle', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('dispatches setVoiceMode when the realtime switch is toggled on', async () => {
    const { store } = renderWithProviders(<VoicePanel />, { initialEntries: ['/settings/voice'] });

    const toggle = await screen.findByTestId('voice-mode-realtime-toggle');
    expect(store.getState().mascot.voiceMode).toBe('classic');

    await act(async () => {
      fireEvent.click(toggle);
    });

    expect(store.getState().mascot.voiceMode).toBe('realtime');
  });
});
