import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { MicCloudComposer } from './MicCloudComposer';

// transcribeCloud + encodeBlobToWav are the network/heavy boundaries — mock
// them here so we can drive the state machine without touching real APIs.
const transcribeCloudMock = vi.fn();
const encodeBlobToWavMock = vi.fn();
vi.mock('./voice/sttClient', () => ({
  transcribeCloud: (...args: unknown[]) => transcribeCloudMock(...args),
}));
vi.mock('./voice/wavEncoder', () => ({
  encodeBlobToWav: (...args: unknown[]) => encodeBlobToWavMock(...args),
}));

interface FakeRecorder {
  state: 'inactive' | 'recording' | 'paused';
  mimeType: string;
  ondataavailable: ((e: { data: Blob }) => void) | null;
  onstop: (() => void) | null;
  start: () => void;
  stop: () => void;
}

function makeFakeRecorder(mime: string): FakeRecorder {
  const rec: FakeRecorder = {
    state: 'inactive',
    mimeType: mime,
    ondataavailable: null,
    onstop: null,
    start() {
      rec.state = 'recording';
    },
    stop() {
      rec.state = 'inactive';
      // Simulate the browser delivering one chunk + the onstop callback.
      rec.ondataavailable?.({ data: new Blob([new Uint8Array([1, 2, 3])], { type: mime }) });
      rec.onstop?.();
    },
  };
  return rec;
}

const fakeStream = { getTracks: () => [{ stop: vi.fn() }] } as unknown as MediaStream;

describe('MicCloudComposer', () => {
  let recorder: FakeRecorder;
  let getUserMediaMock: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    transcribeCloudMock.mockReset();
    encodeBlobToWavMock.mockReset();
    recorder = makeFakeRecorder('audio/webm;codecs=opus');

    getUserMediaMock = vi.fn().mockResolvedValue(fakeStream);
    // jsdom's `navigator` is a real object — stub the property in place so
    // the real prototype chain (React's userAgent reads, etc.) keeps working.
    Object.defineProperty(globalThis.navigator, 'mediaDevices', {
      value: { getUserMedia: getUserMediaMock },
      configurable: true,
      writable: true,
    });

    // `new MediaRecorder(...)` requires a real constructor; `vi.fn(() => x)`
    // returns an object but isn't constructible. Use a class wrapper.
    class FakeRecorderCtor {
      constructor() {
        return recorder as unknown as MediaRecorder;
      }
      static isTypeSupported(m: string) {
        return m.startsWith('audio/webm');
      }
    }
    vi.stubGlobal('MediaRecorder', FakeRecorderCtor);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('renders the idle "Tap and speak" state', () => {
    render(<MicCloudComposer disabled={false} onSubmit={vi.fn()} />);
    expect(screen.getByText('Tap and speak')).toBeInTheDocument();
  });

  it('shows a "Waiting" label when disabled', () => {
    render(<MicCloudComposer disabled={true} onSubmit={vi.fn()} />);
    expect(screen.getByText(/waiting/i)).toBeInTheDocument();
  });

  it('does not start recording when disabled', () => {
    render(<MicCloudComposer disabled={true} onSubmit={vi.fn()} />);
    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    expect(getUserMediaMock).not.toHaveBeenCalled();
  });

  it('starts recording on tap, then transcribes + submits on second tap', async () => {
    transcribeCloudMock.mockResolvedValueOnce('hello world');
    const onSubmit = vi.fn();
    const onError = vi.fn();
    render(<MicCloudComposer disabled={false} onSubmit={onSubmit} onError={onError} />);

    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    await waitFor(() => expect(getUserMediaMock).toHaveBeenCalled());
    expect(onError).not.toHaveBeenCalled();
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /stop recording and send/i })).toBeInTheDocument()
    );
    expect(getUserMediaMock).toHaveBeenCalledWith({
      audio: expect.objectContaining({
        channelCount: 1,
        echoCancellation: true,
        noiseSuppression: true,
        autoGainControl: true,
      }),
    });

    fireEvent.click(screen.getByRole('button', { name: /stop recording and send/i }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledWith('hello world'));
    expect(transcribeCloudMock).toHaveBeenCalledTimes(1);
  });

  it('forwards the language prop to transcribeCloud', async () => {
    transcribeCloudMock.mockResolvedValueOnce('hi');
    render(<MicCloudComposer disabled={false} onSubmit={vi.fn()} language="es" />);
    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /stop recording and send/i })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('button', { name: /stop recording and send/i }));
    await waitFor(() => expect(transcribeCloudMock).toHaveBeenCalled());
    const opts = transcribeCloudMock.mock.calls[0][1];
    expect(opts).toEqual({ language: 'es' });
  });

  it('surfaces a permission-denied error via onError', async () => {
    getUserMediaMock.mockRejectedValueOnce(new Error('NotAllowed'));
    const onError = vi.fn();
    render(<MicCloudComposer disabled={false} onSubmit={vi.fn()} onError={onError} />);
    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    await waitFor(() => expect(onError).toHaveBeenCalledWith(expect.stringMatching(/permission/i)));
  });

  it('falls back to wav re-encode when the native attempt fails', async () => {
    transcribeCloudMock
      .mockRejectedValueOnce(new Error('codec not accepted'))
      .mockResolvedValueOnce('after fallback');
    encodeBlobToWavMock.mockResolvedValueOnce(
      new Blob([new Uint8Array([0])], { type: 'audio/wav' })
    );
    const onSubmit = vi.fn();
    render(<MicCloudComposer disabled={false} onSubmit={onSubmit} />);
    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /stop recording and send/i })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('button', { name: /stop recording and send/i }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledWith('after fallback'));
    expect(encodeBlobToWavMock).toHaveBeenCalledTimes(1);
    expect(transcribeCloudMock).toHaveBeenCalledTimes(2);
  });

  it('reports an error when transcription returns empty text', async () => {
    transcribeCloudMock.mockResolvedValueOnce('');
    const onError = vi.fn();
    const onSubmit = vi.fn();
    render(<MicCloudComposer disabled={false} onSubmit={onSubmit} onError={onError} />);
    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /stop recording and send/i })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('button', { name: /stop recording and send/i }));
    await waitFor(() =>
      expect(onError).toHaveBeenCalledWith(expect.stringMatching(/no speech detected/i))
    );
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it('reports a microphone-unavailable error when getUserMedia is missing', () => {
    Object.defineProperty(globalThis.navigator, 'mediaDevices', {
      value: undefined,
      configurable: true,
      writable: true,
    });
    const onError = vi.fn();
    render(<MicCloudComposer disabled={false} onSubmit={vi.fn()} onError={onError} />);
    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    expect(onError).toHaveBeenCalledWith(expect.stringMatching(/not available/i));
  });
});
