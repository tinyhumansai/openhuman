import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { VoiceNotePlayer } from './VoiceNotePlayer';

const api = vi.hoisted(() => ({ readVoiceNote: vi.fn() }));
const analytics = vi.hoisted(() => ({ trackAnalyticsEvent: vi.fn() }));
vi.mock('./voiceNoteApi', async importOriginal => ({
  ...(await importOriginal<typeof import('./voiceNoteApi')>()),
  readVoiceNote: api.readVoiceNote,
}));
vi.mock('../../components/analytics', () => analytics);

class FakeAudio {
  static last: FakeAudio | null = null;
  paused = true;
  currentTime = 0;
  duration = 12;
  listeners = new Map<string, Array<() => void>>();
  constructor(public src: string) {
    FakeAudio.last = this;
  }
  addEventListener(name: string, cb: () => void) {
    this.listeners.set(name, [...(this.listeners.get(name) ?? []), cb]);
  }
  emit(name: string) {
    for (const cb of this.listeners.get(name) ?? []) cb();
  }
  play() {
    this.paused = false;
    this.emit('play');
    return Promise.resolve();
  }
  pause() {
    this.paused = true;
    this.emit('pause');
  }
}

describe('VoiceNotePlayer', () => {
  beforeEach(() => {
    FakeAudio.last = null;
    api.readVoiceNote.mockReset();
    analytics.trackAnalyticsEvent.mockReset();
    vi.stubGlobal('Audio', FakeAudio);
    URL.createObjectURL = vi.fn(() => 'blob:note');
    URL.revokeObjectURL = vi.fn();
  });
  afterEach(() => vi.unstubAllGlobals());

  it('fetches the audio only on first play, then toggles play and pause', async () => {
    api.readVoiceNote.mockResolvedValue(new Blob([new Uint8Array([1])], { type: 'audio/mpeg' }));
    render(<VoiceNotePlayer noteId="reply-1" durationMs={12000} />);
    expect(api.readVoiceNote).not.toHaveBeenCalled();
    expect(screen.getByTestId('voice-note-time')).toHaveTextContent('0:12');

    fireEvent.click(screen.getByTestId('voice-note-toggle'));
    await waitFor(() => expect(screen.getByLabelText('Pause voice note')).toBeInTheDocument());
    expect(api.readVoiceNote).toHaveBeenCalledWith('reply-1');
    expect(FakeAudio.last?.src).toBe('blob:note');
    expect(analytics.trackAnalyticsEvent).toHaveBeenCalledWith('voice_note_played', {});

    fireEvent.click(screen.getByTestId('voice-note-toggle'));
    await waitFor(() => expect(screen.getByLabelText('Play voice note')).toBeInTheDocument());
    expect(api.readVoiceNote).toHaveBeenCalledTimes(1);
  });

  it('plays a draft straight from its src without asking the core', async () => {
    render(<VoiceNotePlayer src="blob:draft" durationMs={3000} />);
    fireEvent.click(screen.getByTestId('voice-note-toggle'));
    await waitFor(() => expect(FakeAudio.last?.src).toBe('blob:draft'));
    expect(api.readVoiceNote).not.toHaveBeenCalled();
  });

  it('follows playback position and resets when the clip ends', async () => {
    render(<VoiceNotePlayer src="blob:draft" durationMs={10000} />);
    fireEvent.click(screen.getByTestId('voice-note-toggle'));
    await waitFor(() => expect(FakeAudio.last).not.toBeNull());
    const audio = FakeAudio.last!;
    audio.currentTime = 4;
    audio.emit('timeupdate');
    await waitFor(() => expect(screen.getByTestId('voice-note-time')).toHaveTextContent('0:04'));
    audio.emit('ended');
    await waitFor(() => expect(screen.getByTestId('voice-note-time')).toHaveTextContent('0:10'));
    audio.emit('loadedmetadata');
    await waitFor(() => expect(screen.getByTestId('voice-note-time')).toHaveTextContent('0:12'));
  });

  it('drives assistant mascot playback from stored visemes and silences it on pause/unmount', async () => {
    const { unmount } = render(
      <VoiceNotePlayer
        src="blob:reply"
        mascotPlayback
        speechText="reply"
        visemes={[{ viseme: 'aa', start_ms: 0, end_ms: 500 }]}
      />
    );
    const onPlayback = vi.fn();
    window.addEventListener('voice-note-mascot', onPlayback);
    fireEvent.click(screen.getByTestId('voice-note-toggle'));
    await waitFor(() => expect(onPlayback).toHaveBeenCalled());
    const owner = onPlayback.mock.calls.at(-1)?.[0].detail.owner;
    expect(onPlayback.mock.calls.at(-1)?.[0].detail).toMatchObject({ active: true, viseme: 'aa' });
    fireEvent.click(screen.getByTestId('voice-note-toggle'));
    await waitFor(() =>
      expect(onPlayback.mock.calls.at(-1)?.[0].detail).toMatchObject({ owner, active: false })
    );
    fireEvent.click(screen.getByTestId('voice-note-toggle'));
    await waitFor(() =>
      expect(onPlayback.mock.calls.at(-1)?.[0].detail).toMatchObject({ owner, active: true })
    );
    unmount();
    expect(onPlayback.mock.calls.at(-1)?.[0].detail).toMatchObject({ owner, active: false });
    window.removeEventListener('voice-note-mascot', onPlayback);
  });

  it('keeps mascot playback silenced if a paused audio element reports a late timeupdate', async () => {
    const onPlayback = vi.fn();
    window.addEventListener('voice-note-mascot', onPlayback);
    render(
      <VoiceNotePlayer
        src="blob:reply"
        mascotPlayback
        visemes={[{ viseme: 'aa', start_ms: 0, end_ms: 500 }]}
      />
    );
    fireEvent.click(screen.getByTestId('voice-note-toggle'));
    await waitFor(() => expect(onPlayback.mock.calls.at(-1)?.[0].detail.active).toBe(true));
    const audio = FakeAudio.last!;
    audio.pause();
    audio.currentTime = 0.2;
    audio.emit('timeupdate');
    expect(onPlayback.mock.calls.at(-1)?.[0].detail.active).toBe(false);
    window.removeEventListener('voice-note-mascot', onPlayback);
  });

  it('samples reply visemes during playback between media timeupdate events', async () => {
    let frame: FrameRequestCallback | undefined;
    const cancel = vi.fn();
    vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
      frame = callback;
      return 1;
    });
    vi.stubGlobal('cancelAnimationFrame', cancel);
    const onPlayback = vi.fn();
    window.addEventListener('voice-note-mascot', onPlayback);
    render(
      <VoiceNotePlayer
        src="blob:reply"
        mascotPlayback
        visemes={[
          { viseme: 'aa', start_ms: 0, end_ms: 100 },
          { viseme: 'PP', start_ms: 100, end_ms: 500 },
        ]}
      />
    );
    fireEvent.click(screen.getByTestId('voice-note-toggle'));
    await waitFor(() => expect(frame).toBeTypeOf('function'));
    const audio = FakeAudio.last!;
    audio.currentTime = 0.25;
    act(() => frame?.(16));
    expect(onPlayback.mock.calls.at(-1)?.[0].detail).toMatchObject({ active: true, viseme: 'PP' });
    audio.pause();
    expect(cancel).toHaveBeenCalled();
    window.removeEventListener('voice-note-mascot', onPlayback);
  });

  it('shows an error when the core has no audio for the note', async () => {
    api.readVoiceNote.mockResolvedValue(null);
    render(<VoiceNotePlayer noteId="missing" />);
    fireEvent.click(screen.getByTestId('voice-note-toggle'));
    expect(await screen.findByRole('alert')).toHaveTextContent('Could not load this voice note.');
  });

  it('discloses the transcript on request and says so when it is empty', () => {
    const { rerender } = render(<VoiceNotePlayer src="blob:x" transcript="meet at noon" />);
    expect(screen.queryByTestId('voice-note-transcript')).toBeNull();
    fireEvent.click(screen.getByTestId('voice-note-transcript-toggle'));
    expect(screen.getByTestId('voice-note-transcript')).toHaveTextContent('meet at noon');
    rerender(<VoiceNotePlayer src="blob:x" transcript="  " />);
    expect(screen.getByTestId('voice-note-transcript')).toHaveTextContent(
      'No transcript available'
    );
  });

  it('draws one bar per waveform sample', () => {
    render(<VoiceNotePlayer src="blob:x" waveform={[0.1, 0.5, 1]} />);
    expect(screen.getByTestId('voice-note-waveform').children).toHaveLength(3);
  });
});
