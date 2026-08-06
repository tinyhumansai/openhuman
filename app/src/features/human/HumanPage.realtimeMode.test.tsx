/**
 * Unit tests for HumanPage's realtime voice overlay gate (#5399). The overlay
 * renders only when the build flag is on AND the persisted mascot voice mode is
 * `realtime`; the classic push-to-talk path is always present. Config is mocked
 * with the flag ON here (the global setup mock ships it OFF), and
 * RealtimeVoiceControls is stubbed so the ElevenLabs SDK never loads.
 */
import { configureStore } from '@reduxjs/toolkit';
import { render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import chatRuntimeReducer from '../../store/chatRuntimeSlice';
import mascotReducer, { setVoiceMode } from '../../store/mascotSlice';
import threadReducer from '../../store/threadSlice';
import HumanPage from './HumanPage';

// Flip the realtime gate ON for this file (global setup ships it OFF). Spread
// the real module so every other config export keeps its production value.
vi.mock('../../utils/config', async () => {
  const actual = await vi.importActual<typeof import('../../utils/config')>('../../utils/config');
  return { ...actual, VOICE_MODE_FLAG_ENABLED: true };
});

// Stub the overlay so the ElevenLabs `ConversationProvider`/SDK never mounts —
// this test only pins the render gate, not the controls (covered separately).
vi.mock('./RealtimeVoiceControls', () => ({
  default: () => <div data-testid="realtime-voice-controls-stub" />,
}));

vi.mock('../conversations/Conversations', () => ({
  default: () => <div data-testid="conversations-stub" />,
}));

vi.mock('./Mascot', async importOriginal => {
  const actual = await importOriginal<typeof import('./Mascot')>();
  return {
    ...actual,
    RiveMascot: () => <div data-testid="mascot-stub" />,
    CustomGifMascot: () => <img data-testid="custom-gif-mascot" alt="" />,
  };
});

vi.mock('./useHumanMascot', () => ({ useHumanMascot: () => ({ face: 'idle', visemes: [] }) }));
vi.mock('./Mascot/manifest/useMascotManifest', () => ({
  useMascotManifest: () => ({ manifest: null, entry: null, loading: false, error: null }),
}));

function renderWithVoiceMode(mode: 'classic' | 'realtime') {
  const store = configureStore({
    reducer: { mascot: mascotReducer, thread: threadReducer, chatRuntime: chatRuntimeReducer },
  });
  store.dispatch(setVoiceMode(mode));
  return render(
    <Provider store={store}>
      <HumanPage />
    </Provider>
  );
}

describe('HumanPage — realtime voice overlay gate', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('renders the realtime controls when voice mode is realtime and the flag is on', () => {
    renderWithVoiceMode('realtime');
    expect(screen.getByTestId('realtime-voice-controls-stub')).toBeInTheDocument();
  });

  it('hides the realtime controls when voice mode is classic', () => {
    renderWithVoiceMode('classic');
    expect(screen.queryByTestId('realtime-voice-controls-stub')).not.toBeInTheDocument();
  });
});
