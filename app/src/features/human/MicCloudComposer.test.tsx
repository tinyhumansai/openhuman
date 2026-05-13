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
  // Snapshot the descriptor so afterEach can restore it — without this, the
  // first test that overrides `navigator.mediaDevices` leaks the override
  // into siblings and makes the suite order-dependent.
  let originalMediaDevicesDescriptor: PropertyDescriptor | undefined;

  beforeEach(() => {
    originalMediaDevicesDescriptor = Object.getOwnPropertyDescriptor(
      globalThis.navigator,
      'mediaDevices'
    );
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
    if (originalMediaDevicesDescriptor) {
      Object.defineProperty(globalThis.navigator, 'mediaDevices', originalMediaDevicesDescriptor);
    } else {
      delete (globalThis.navigator as { mediaDevices?: MediaDevices }).mediaDevices;
    }
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

  it('surfaces a permission-denied error via onError for NotAllowedError', async () => {
    const err = Object.assign(new DOMException('', 'NotAllowedError'));
    getUserMediaMock.mockRejectedValueOnce(err);
    const onError = vi.fn();
    render(<MicCloudComposer disabled={false} onSubmit={vi.fn()} onError={onError} />);
    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    await waitFor(() => expect(onError).toHaveBeenCalledWith(expect.stringMatching(/permission/i)));
  });

  it('surfaces a device-unavailable error for OverconstrainedError', async () => {
    const err = new DOMException('', 'OverconstrainedError');
    getUserMediaMock.mockRejectedValueOnce(err);
    const onError = vi.fn();
    render(<MicCloudComposer disabled={false} onSubmit={vi.fn()} onError={onError} />);
    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    await waitFor(() =>
      expect(onError).toHaveBeenCalledWith(expect.stringMatching(/unavailable/i))
    );
  });

  it('surfaces an in-use error for NotReadableError', async () => {
    const err = new DOMException('', 'NotReadableError');
    getUserMediaMock.mockRejectedValueOnce(err);
    const onError = vi.fn();
    render(<MicCloudComposer disabled={false} onSubmit={vi.fn()} onError={onError} />);
    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    await waitFor(() => expect(onError).toHaveBeenCalledWith(expect.stringMatching(/in use/i)));
  });

  it('surfaces a generic error for non-DOMException getUserMedia failures', async () => {
    getUserMediaMock.mockRejectedValueOnce(new Error('some other error'));
    const onError = vi.fn();
    render(<MicCloudComposer disabled={false} onSubmit={vi.fn()} onError={onError} />);
    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    await waitFor(() =>
      expect(onError).toHaveBeenCalledWith(expect.stringMatching(/microphone error/i))
    );
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

  // ── Spacebar shortcut (#1471) ────────────────────────────────────────────

  it('spacebar starts recording when idle and stops + submits on second press', async () => {
    transcribeCloudMock.mockResolvedValueOnce('voice via space');
    const onSubmit = vi.fn();
    render(<MicCloudComposer disabled={false} onSubmit={onSubmit} />);

    fireEvent.keyDown(window, { code: 'Space' });
    await waitFor(() => expect(getUserMediaMock).toHaveBeenCalled());
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /stop recording and send/i })).toBeInTheDocument()
    );

    fireEvent.keyDown(window, { code: 'Space' });
    await waitFor(() => expect(onSubmit).toHaveBeenCalledWith('voice via space'));
  });

  it('spacebar ignores key repeat so holding the key does not flap the recorder', () => {
    render(<MicCloudComposer disabled={false} onSubmit={vi.fn()} />);
    fireEvent.keyDown(window, { code: 'Space', repeat: true });
    expect(getUserMediaMock).not.toHaveBeenCalled();
  });

  it('spacebar ignores modifier combinations so Shift-Space etc. stay free', () => {
    render(<MicCloudComposer disabled={false} onSubmit={vi.fn()} />);
    fireEvent.keyDown(window, { code: 'Space', shiftKey: true });
    fireEvent.keyDown(window, { code: 'Space', ctrlKey: true });
    fireEvent.keyDown(window, { code: 'Space', metaKey: true });
    fireEvent.keyDown(window, { code: 'Space', altKey: true });
    expect(getUserMediaMock).not.toHaveBeenCalled();
  });

  it('spacebar does not trigger when focus is inside a text input', () => {
    render(
      <>
        <input data-testid="text-field" type="text" />
        <MicCloudComposer disabled={false} onSubmit={vi.fn()} />
      </>
    );
    const input = screen.getByTestId('text-field');
    input.focus();
    fireEvent.keyDown(input, { code: 'Space' });
    expect(getUserMediaMock).not.toHaveBeenCalled();
  });

  it('spacebar does not trigger when focus is inside a textarea', () => {
    render(
      <>
        <textarea data-testid="ta" />
        <MicCloudComposer disabled={false} onSubmit={vi.fn()} />
      </>
    );
    const ta = screen.getByTestId('ta');
    ta.focus();
    fireEvent.keyDown(ta, { code: 'Space' });
    expect(getUserMediaMock).not.toHaveBeenCalled();
  });

  it('spacebar does not trigger when focus is inside a contenteditable surface', () => {
    render(
      <>
        <div data-testid="ce" contentEditable suppressContentEditableWarning>
          x
        </div>
        <MicCloudComposer disabled={false} onSubmit={vi.fn()} />
      </>
    );
    const ce = screen.getByTestId('ce');
    ce.focus();
    fireEvent.keyDown(ce, { code: 'Space' });
    expect(getUserMediaMock).not.toHaveBeenCalled();
  });

  it('spacebar is a no-op while the composer is disabled', () => {
    render(<MicCloudComposer disabled={true} onSubmit={vi.fn()} />);
    fireEvent.keyDown(window, { code: 'Space' });
    expect(getUserMediaMock).not.toHaveBeenCalled();
  });

  it('removes the window keydown listener on unmount', () => {
    const removeSpy = vi.spyOn(window, 'removeEventListener');
    const { unmount } = render(<MicCloudComposer disabled={false} onSubmit={vi.fn()} />);
    unmount();
    expect(removeSpy).toHaveBeenCalledWith('keydown', expect.any(Function));
    removeSpy.mockRestore();
  });

  // ── Device selector (showDeviceSelector) ─────────────────────────────────

  it('enumerates devices on mount when showDeviceSelector is true', async () => {
    const enumerateDevicesMock = vi.fn().mockResolvedValue([
      { kind: 'audioinput', deviceId: 'dev1', label: 'Built-in Mic' },
      { kind: 'audioinput', deviceId: 'dev2', label: 'USB Headset' },
      { kind: 'videoinput', deviceId: 'cam1', label: 'Camera' },
    ]);
    Object.defineProperty(globalThis.navigator, 'mediaDevices', {
      value: { getUserMedia: getUserMediaMock, enumerateDevices: enumerateDevicesMock },
      configurable: true,
      writable: true,
    });

    render(<MicCloudComposer disabled={false} onSubmit={vi.fn()} showDeviceSelector />);

    await waitFor(() => expect(enumerateDevicesMock).toHaveBeenCalled());
    expect(await screen.findByRole('combobox', { name: /microphone device/i })).toBeInTheDocument();
    expect(screen.getByText('Built-in Mic')).toBeInTheDocument();
    expect(screen.getByText('USB Headset')).toBeInTheDocument();
    // Video input must not appear
    expect(screen.queryByText('Camera')).not.toBeInTheDocument();
  });

  it('does not show the selector when showDeviceSelector is false (default)', async () => {
    const enumerateDevicesMock = vi.fn().mockResolvedValue([
      { kind: 'audioinput', deviceId: 'dev1', label: 'Built-in Mic' },
      { kind: 'audioinput', deviceId: 'dev2', label: 'USB Headset' },
    ]);
    Object.defineProperty(globalThis.navigator, 'mediaDevices', {
      value: { getUserMedia: getUserMediaMock, enumerateDevices: enumerateDevicesMock },
      configurable: true,
      writable: true,
    });

    render(<MicCloudComposer disabled={false} onSubmit={vi.fn()} />);

    await waitFor(() => {
      expect(
        screen.queryByRole('combobox', { name: /microphone device/i })
      ).not.toBeInTheDocument();
      expect(enumerateDevicesMock).not.toHaveBeenCalled();
    });
  });

  it('shows the selector disabled when only one device is available', async () => {
    const enumerateDevicesMock = vi
      .fn()
      .mockResolvedValue([{ kind: 'audioinput', deviceId: 'dev1', label: 'Built-in Mic' }]);
    Object.defineProperty(globalThis.navigator, 'mediaDevices', {
      value: { getUserMedia: getUserMediaMock, enumerateDevices: enumerateDevicesMock },
      configurable: true,
      writable: true,
    });

    render(<MicCloudComposer disabled={false} onSubmit={vi.fn()} showDeviceSelector />);

    const select = await screen.findByRole('combobox', { name: /microphone device/i });
    expect(select).toBeInTheDocument();
    expect(select).toBeDisabled();
  });

  it('falls back to "Microphone N" label when browser hides labels before permission', async () => {
    const enumerateDevicesMock = vi.fn().mockResolvedValue([
      { kind: 'audioinput', deviceId: 'dev1', label: '' },
      { kind: 'audioinput', deviceId: 'dev2', label: '' },
    ]);
    Object.defineProperty(globalThis.navigator, 'mediaDevices', {
      value: { getUserMedia: getUserMediaMock, enumerateDevices: enumerateDevicesMock },
      configurable: true,
      writable: true,
    });

    render(<MicCloudComposer disabled={false} onSubmit={vi.fn()} showDeviceSelector />);

    await waitFor(() => expect(screen.queryByRole('combobox')).toBeInTheDocument());
    expect(screen.getByText('Microphone 1')).toBeInTheDocument();
    expect(screen.getByText('Microphone 2')).toBeInTheDocument();
  });

  it('passes the selected deviceId as an exact constraint to getUserMedia', async () => {
    const enumerateDevicesMock = vi.fn().mockResolvedValue([
      { kind: 'audioinput', deviceId: 'dev1', label: 'Built-in Mic' },
      { kind: 'audioinput', deviceId: 'dev2', label: 'USB Headset' },
    ]);
    transcribeCloudMock.mockResolvedValueOnce('hello');
    Object.defineProperty(globalThis.navigator, 'mediaDevices', {
      value: { getUserMedia: getUserMediaMock, enumerateDevices: enumerateDevicesMock },
      configurable: true,
      writable: true,
    });

    render(<MicCloudComposer disabled={false} onSubmit={vi.fn()} showDeviceSelector />);

    // Wait for the selector to appear and pick the second device
    const select = await screen.findByRole('combobox', { name: /microphone device/i });
    fireEvent.change(select, { target: { value: 'dev2' } });

    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    await waitFor(() => expect(getUserMediaMock).toHaveBeenCalled());

    expect(getUserMediaMock).toHaveBeenCalledWith(
      expect.objectContaining({ audio: expect.objectContaining({ deviceId: { exact: 'dev2' } }) })
    );
  });

  it('refreshes device labels after getUserMedia permission is granted', async () => {
    const enumerateDevicesMock = vi
      .fn()
      // First call (on mount): labels hidden
      .mockResolvedValueOnce([
        { kind: 'audioinput', deviceId: 'dev1', label: '' },
        { kind: 'audioinput', deviceId: 'dev2', label: '' },
      ])
      // Second call (post-permission): real labels
      .mockResolvedValueOnce([
        { kind: 'audioinput', deviceId: 'dev1', label: 'Built-in Mic' },
        { kind: 'audioinput', deviceId: 'dev2', label: 'USB Headset' },
      ]);
    transcribeCloudMock.mockResolvedValueOnce('ok');
    Object.defineProperty(globalThis.navigator, 'mediaDevices', {
      value: { getUserMedia: getUserMediaMock, enumerateDevices: enumerateDevicesMock },
      configurable: true,
      writable: true,
    });

    render(<MicCloudComposer disabled={false} onSubmit={vi.fn()} showDeviceSelector />);

    // Mount enumerate ran — labels are blank placeholders
    await waitFor(() => expect(screen.queryByRole('combobox')).toBeInTheDocument());
    expect(screen.getByText('Microphone 1')).toBeInTheDocument();

    // Start recording → triggers the post-permission refresh
    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /stop recording and send/i })).toBeInTheDocument()
    );

    // After permission, real labels should now be visible
    await waitFor(() => expect(screen.getByText('Built-in Mic')).toBeInTheDocument());
    expect(screen.getByText('USB Headset')).toBeInTheDocument();
  });

  it('handles enumerateDevices throwing gracefully (no crash, selector hidden)', async () => {
    const enumerateDevicesMock = vi.fn().mockRejectedValue(new Error('NotAllowed'));
    Object.defineProperty(globalThis.navigator, 'mediaDevices', {
      value: { getUserMedia: getUserMediaMock, enumerateDevices: enumerateDevicesMock },
      configurable: true,
      writable: true,
    });

    render(<MicCloudComposer disabled={false} onSubmit={vi.fn()} showDeviceSelector />);

    await waitFor(() => expect(enumerateDevicesMock).toHaveBeenCalled());
    // Selector requires >1 device; error yields 0 → selector stays hidden
    expect(screen.queryByRole('combobox', { name: /microphone device/i })).not.toBeInTheDocument();
    // Composer still functional
    expect(screen.getByText('Tap and speak')).toBeInTheDocument();
  });
});
