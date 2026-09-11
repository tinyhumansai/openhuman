/**
 * Unit test for the Human tab's voice entry point (#5399). The realtime
 * "Start voice chat" control now lives in the chat card's composer slot — the
 * one the classic push-to-talk mic used to own — and which of the two renders is
 * decided by two build flags. This pins the wiring from those flags through to
 * the props HumanPage hands Conversations; the controls themselves and the
 * precedence rule are covered separately (RealtimeVoiceControls.test.tsx,
 * voiceEntry.test.ts). RealtimeVoiceControls is stubbed so the ElevenLabs SDK
 * never loads.
 */
import { configureStore } from '@reduxjs/toolkit';
import { render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import chatRuntimeReducer from '../../store/chatRuntimeSlice';
import mascotReducer from '../../store/mascotSlice';
import threadReducer from '../../store/threadSlice';

const flags = { realtimeEnabled: true, showBoth: false };

// The global test setup mocks the whole config module, so override just the two
// flags this file drives — read through getters so a test can flip them between
// renders without re-importing the module.
vi.mock('../../utils/config', async importOriginal => {
  const actual = await importOriginal<Record<string, unknown>>();
  return {
    ...actual,
    get HUMAN_VOICE_REALTIME_ENABLED() {
      return flags.realtimeEnabled;
    },
    get HUMAN_VOICE_SHOW_BOTH() {
      return flags.showBoth;
    },
  };
});

vi.mock('./RealtimeVoiceControls', () => ({
  default: () => <div data-testid="realtime-voice-controls-stub" />,
}));

// Render the slot props so the test observes what the card would actually show,
// rather than asserting on prop identity.
vi.mock('../conversations/Conversations', () => ({
  default: ({
    voiceChatControl,
    showMicComposer,
  }: {
    voiceChatControl?: React.ReactNode;
    showMicComposer?: boolean;
  }) => (
    <div data-testid="conversations-stub">
      {voiceChatControl}
      {showMicComposer && <div data-testid="mic-composer-stub" />}
    </div>
  ),
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

async function renderPage() {
  const { default: HumanPage } = await import('./HumanPage');
  const store = configureStore({
    reducer: { mascot: mascotReducer, thread: threadReducer, chatRuntime: chatRuntimeReducer },
  });
  // HumanPage routes (the composer's idle action opens /human, and the page
  // navigates back), so it needs a router in scope.
  return render(
    <Provider store={store}>
      <MemoryRouter initialEntries={['/human']}>
        <HumanPage />
      </MemoryRouter>
    </Provider>
  );
}

describe('HumanPage — voice entry point', () => {
  beforeEach(() => {
    localStorage.clear();
    flags.realtimeEnabled = true;
    flags.showBoth = false;
  });

  it('shows the realtime control in place of the mic composer by default', async () => {
    await renderPage();
    expect(screen.getByTestId('realtime-voice-controls-stub')).toBeInTheDocument();
    expect(screen.queryByTestId('mic-composer-stub')).not.toBeInTheDocument();
  });

  /**
   * The page is a mascot-only stage now — it no longer embeds the chat rail, so
   * there is no composer to fall back to. Push-to-talk lived in that rail's
   * composer and submitted through its send path, so a
   * `VITE_HUMAN_VOICE_REALTIME=false` build simply has no control to show here.
   */
  it('shows a mascot-only stage with no voice control when realtime is off', async () => {
    flags.realtimeEnabled = false;
    await renderPage();
    expect(screen.queryByTestId('realtime-voice-controls-stub')).not.toBeInTheDocument();
    expect(screen.queryByTestId('mic-composer-stub')).not.toBeInTheDocument();
  });

  // Comparison mode used to show the realtime control beside the chat rail's
  // tap-and-speak composer. With the rail gone there is only the one control,
  // and the assertion that matters is that show-both does not double it.
  it('still shows a single realtime control when the show-both flag is on', async () => {
    flags.showBoth = true;
    await renderPage();
    expect(screen.getAllByTestId('realtime-voice-controls-stub')).toHaveLength(1);
    expect(screen.queryByTestId('mic-composer-stub')).not.toBeInTheDocument();
  });

  // Whichever mode is on, exactly one realtime control exists: the single-control
  // modes put it in the card, comparison mode floats it — never both at once.
  it.each([
    ['realtime', { realtimeEnabled: true, showBoth: false }],
    ['both', { realtimeEnabled: true, showBoth: true }],
  ])('renders the realtime control exactly once in %s mode', async (_label, next) => {
    Object.assign(flags, next);
    await renderPage();
    expect(screen.getAllByTestId('realtime-voice-controls-stub')).toHaveLength(1);
  });
});
