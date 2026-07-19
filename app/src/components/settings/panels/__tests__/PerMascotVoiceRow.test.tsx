import { configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import mascotReducer, { setMascotVoice } from '../../../../store/mascotSlice';
import PerMascotVoiceRow from '../PerMascotVoiceRow';

const { mockSynthesizeSpeech } = vi.hoisted(() => ({ mockSynthesizeSpeech: vi.fn() }));

vi.mock('../../../../features/human/voice/ttsClient', () => ({
  synthesizeSpeech: (...args: unknown[]) => mockSynthesizeSpeech(...args),
}));

const TEST_ID = 'mascot-voice-primary';
const MASCOT_ID = 'yellow';

function buildStore() {
  return configureStore({ reducer: { mascot: mascotReducer } });
}

function renderRow(store = buildStore()) {
  return {
    store,
    ...render(
      <Provider store={store}>
        <PerMascotVoiceRow mascotId={MASCOT_ID} label="Yellow" testIdPrefix={TEST_ID} />
      </Provider>
    ),
  };
}

/** Resolve helper to control the timing of the mocked synthesizeSpeech. */
function deferredTts() {
  let resolve!: (v: { audio_mime: string; audio_base64: string }) => void;
  let reject!: (err: unknown) => void;
  const promise = new Promise<{ audio_mime: string; audio_base64: string }>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe('PerMascotVoiceRow', () => {
  let playSpy: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    vi.clearAllMocks();
    playSpy = vi.fn().mockResolvedValue(undefined);
    // Stub window.Audio so `new window.Audio(src)` records the src and exposes
    // a spy-able play()/pause() without touching the real audio pipeline.
    vi.stubGlobal(
      'Audio',
      class {
        src: string;
        constructor(src?: string) {
          this.src = src ?? '';
        }
        play = playSpy;
        pause = vi.fn();
      }
    );
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('reveals the paste input + Save when the custom option is selected and saves a new id', () => {
    const { store } = renderRow();

    // No custom input until the __custom__ option is chosen.
    expect(screen.queryByTestId(`${TEST_ID}-input`)).not.toBeInTheDocument();

    fireEvent.change(screen.getByTestId(`${TEST_ID}-select`), { target: { value: '__custom__' } });

    const input = screen.getByTestId(`${TEST_ID}-input`);
    expect(input).toBeInTheDocument();

    const saveBtn = screen.getByTestId(`${TEST_ID}-save-paste`);
    // Empty draft still equals the (empty) stored override → disabled.
    expect(saveBtn).toBeDisabled();

    // A whitespace-padded id enables Save and is trimmed on dispatch.
    fireEvent.change(input, { target: { value: '  my-custom-voice  ' } });
    expect(saveBtn).not.toBeDisabled();
    fireEvent.click(saveBtn);

    expect(store.getState().mascot.mascotVoices[MASCOT_ID]).toBe('my-custom-voice');
  });

  it('plays a data:audio preview and toggles the previewing label when synthesizeSpeech resolves', async () => {
    const d = deferredTts();
    mockSynthesizeSpeech.mockReturnValue(d.promise);
    renderRow();

    const previewBtn = screen.getByTestId(`${TEST_ID}-preview`);
    fireEvent.click(previewBtn);

    // Previewing state toggles on immediately (button disabled + label swap).
    await waitFor(() => expect(previewBtn).toBeDisabled());
    expect(previewBtn).toHaveTextContent('Previewing…');

    d.resolve({ audio_mime: 'audio/mpeg', audio_base64: 'QUJD' });

    await waitFor(() => expect(playSpy).toHaveBeenCalledTimes(1));
    // A data:audio URI was constructed from the mime + base64 payload.
    expect(playSpy.mock.instances[0].src).toBe('data:audio/mpeg;base64,QUJD');

    // Previewing resets once the preview finishes.
    await waitFor(() => expect(previewBtn).not.toBeDisabled());
    expect(previewBtn).toHaveTextContent('Preview voice');
  });

  it('renders the preview-error text and resets previewing when synthesizeSpeech rejects', async () => {
    const d = deferredTts();
    mockSynthesizeSpeech.mockReturnValue(d.promise);
    renderRow();

    const previewBtn = screen.getByTestId(`${TEST_ID}-preview`);
    fireEvent.click(previewBtn);
    await waitFor(() => expect(previewBtn).toBeDisabled());

    d.reject(new Error('tts exploded'));

    const errorBox = await screen.findByTestId(`${TEST_ID}-preview-error`);
    expect(errorBox).toHaveTextContent('tts exploded');
    expect(playSpy).not.toHaveBeenCalled();

    // finally branch clears the previewing state.
    await waitFor(() => expect(previewBtn).not.toBeDisabled());
    expect(previewBtn).toHaveTextContent('Preview voice');
  });

  it('is a no-op when a preview resolves after the row has unmounted (previewRequestIdRef guard)', async () => {
    const d = deferredTts();
    mockSynthesizeSpeech.mockReturnValue(d.promise);
    const { unmount } = renderRow();

    fireEvent.click(screen.getByTestId(`${TEST_ID}-preview`));
    await waitFor(() => expect(mockSynthesizeSpeech).toHaveBeenCalledTimes(1));

    // Unmount bumps previewRequestIdRef, so the late resolve loses the race.
    unmount();
    d.resolve({ audio_mime: 'audio/mpeg', audio_base64: 'QUJD' });

    // Give the resolved promise a microtask/macrotask to flush.
    await new Promise(r => setTimeout(r, 0));
    expect(playSpy).not.toHaveBeenCalled();
  });

  it('exposes the current voice id set via setMascotVoice', () => {
    const store = buildStore();
    store.dispatch(setMascotVoice({ mascotId: MASCOT_ID, voiceId: 'pNInz6obpgDQGcFmaJgB' }));
    renderRow(store);
    expect(screen.getByTestId(`${TEST_ID}-current`)).toHaveTextContent('pNInz6obpgDQGcFmaJgB');
  });

  it('dispatches setMascotVoice when a curated preset is picked from the dropdown', () => {
    const { store } = renderRow();
    fireEvent.change(screen.getByTestId(`${TEST_ID}-select`), {
      target: { value: 'pNInz6obpgDQGcFmaJgB' },
    });
    expect(store.getState().mascot.mascotVoices[MASCOT_ID]).toBe('pNInz6obpgDQGcFmaJgB');
  });

  it('clears the per-mascot override via the reset button', () => {
    const store = buildStore();
    store.dispatch(setMascotVoice({ mascotId: MASCOT_ID, voiceId: 'pNInz6obpgDQGcFmaJgB' }));
    renderRow(store);

    const resetBtn = screen.getByTestId(`${TEST_ID}-reset`);
    expect(resetBtn).not.toBeDisabled();
    fireEvent.click(resetBtn);

    expect(store.getState().mascot.mascotVoices[MASCOT_ID]).toBeUndefined();
  });

  it('stops the prior preview audio when a second preview starts', async () => {
    const first = deferredTts();
    const second = deferredTts();
    mockSynthesizeSpeech.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    renderRow();

    const previewBtn = screen.getByTestId(`${TEST_ID}-preview`);

    // First preview resolves and plays, seeding previewAudioRef.
    fireEvent.click(previewBtn);
    first.resolve({ audio_mime: 'audio/mpeg', audio_base64: 'QUJD' });
    await waitFor(() => expect(playSpy).toHaveBeenCalledTimes(1));
    const firstAudio = playSpy.mock.instances[0];
    await waitFor(() => expect(previewBtn).not.toBeDisabled());

    // Second preview should pause + clear the prior audio before synthesizing.
    fireEvent.click(previewBtn);
    await waitFor(() => expect(mockSynthesizeSpeech).toHaveBeenCalledTimes(2));
    expect(firstAudio.pause).toHaveBeenCalled();
    expect(firstAudio.src).toBe('');

    second.resolve({ audio_mime: 'audio/mpeg', audio_base64: 'WFla' });
    await waitFor(() => expect(playSpy).toHaveBeenCalledTimes(2));
  });
});
