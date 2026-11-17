import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { reduceWaveform } from './useVoiceNoteRecorder';
import { VoiceNoteComposer } from './VoiceNoteComposer';

vi.mock('../../components/analytics', () => ({ trackAnalyticsEvent: vi.fn() }));

class FakeRecorder {
  static instances: FakeRecorder[] = [];
  static isTypeSupported = () => true;
  state: 'inactive' | 'recording' = 'inactive';
  mimeType = 'audio/webm';
  private listeners = new Map<string, Array<(e: unknown) => void>>();
  constructor(public stream: MediaStream) {
    FakeRecorder.instances.push(this);
  }
  addEventListener(name: string, cb: (e: unknown) => void) {
    this.listeners.set(name, [...(this.listeners.get(name) ?? []), cb]);
  }
  start() {
    this.state = 'recording';
  }
  stop() {
    this.state = 'inactive';
    for (const cb of this.listeners.get('dataavailable') ?? []) {
      cb({ data: new Blob([new Uint8Array([1, 2, 3])], { type: 'audio/webm' }) });
    }
    for (const cb of this.listeners.get('stop') ?? []) cb({});
  }
}

const trackStop = vi.fn();

describe('VoiceNoteComposer', () => {
  beforeEach(() => {
    FakeRecorder.instances = [];
    trackStop.mockReset();
    vi.stubGlobal('MediaRecorder', FakeRecorder);
    Object.defineProperty(navigator, 'mediaDevices', {
      configurable: true,
      value: {
        getUserMedia: vi.fn().mockResolvedValue({ getTracks: () => [{ stop: trackStop }] }),
      },
    });
    URL.createObjectURL = vi.fn(() => 'blob:draft');
    URL.revokeObjectURL = vi.fn();
  });
  afterEach(() => vi.unstubAllGlobals());

  async function recordClip() {
    fireEvent.click(screen.getByTestId('voice-note-record'));
    await screen.findByTestId('voice-note-recording');
    act(() => fireEvent.click(screen.getByTestId('voice-note-stop')));
    await screen.findByTestId('voice-note-preview');
  }

  it('records, previews and sends the clip, then returns to idle', async () => {
    const onSend = vi.fn().mockResolvedValue(undefined);
    render(<VoiceNoteComposer onSend={onSend} />);
    await recordClip();
    expect(trackStop).toHaveBeenCalled();

    fireEvent.click(screen.getByTestId('voice-note-send'));
    await waitFor(() => expect(onSend).toHaveBeenCalledTimes(1));
    const draft = onSend.mock.calls[0][0];
    expect(draft.blob.size).toBe(3);
    expect(draft.durationMs).toBeGreaterThanOrEqual(0);
    await screen.findByTestId('voice-note-record');
  });

  it('keeps the draft and reports the error when sending fails', async () => {
    const onError = vi.fn();
    render(
      <VoiceNoteComposer onSend={vi.fn().mockRejectedValue(new Error('boom'))} onError={onError} />
    );
    await recordClip();
    fireEvent.click(screen.getByTestId('voice-note-send'));
    await waitFor(() => expect(onError).toHaveBeenCalledWith('boom'));
    expect(screen.getByTestId('voice-note-preview')).toBeInTheDocument();
  });

  it('discards a recording in progress and releases the microphone', async () => {
    render(<VoiceNoteComposer onSend={vi.fn()} />);
    fireEvent.click(screen.getByTestId('voice-note-record'));
    await screen.findByTestId('voice-note-recording');
    fireEvent.click(screen.getByTestId('voice-note-discard'));
    await screen.findByTestId('voice-note-record');
    expect(trackStop).toHaveBeenCalled();
  });

  it('discards a finished draft without sending it', async () => {
    const onSend = vi.fn();
    render(<VoiceNoteComposer onSend={onSend} />);
    await recordClip();
    fireEvent.click(screen.getByTestId('voice-note-discard'));
    await screen.findByTestId('voice-note-record');
    expect(onSend).not.toHaveBeenCalled();
  });

  it('reports a denied microphone', async () => {
    (navigator.mediaDevices.getUserMedia as ReturnType<typeof vi.fn>).mockRejectedValue(
      new Error('denied')
    );
    const onError = vi.fn();
    render(<VoiceNoteComposer onSend={vi.fn()} onError={onError} />);
    fireEvent.click(screen.getByTestId('voice-note-record'));
    await waitFor(() =>
      expect(onError).toHaveBeenCalledWith('Microphone access was denied or is unavailable.')
    );
    expect(screen.getByTestId('voice-note-record')).toBeInTheDocument();
  });

  it('disables recording while the composer is blocked', () => {
    render(<VoiceNoteComposer disabled onSend={vi.fn()} />);
    expect(screen.getByTestId('voice-note-record')).toBeDisabled();
  });

  it('resamples amplitude samples to a fixed bar count in 0..1', () => {
    expect(reduceWaveform([])).toEqual([]);
    const bars = reduceWaveform([0, 0.5, 2, 0.25], 2);
    expect(bars).toEqual([0.5, 1]);
  });
});
