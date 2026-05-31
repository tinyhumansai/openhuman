import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  isLowConfidenceTranscript,
  MAX_RECORDING_MS,
  MicComposer,
  STT_MAX_RETRIES,
} from './MicComposer';

// transcribeWithFactory + encodeBlobToWav are the network/heavy boundaries —
// mock them here so we can drive the state machine without touching real APIs.
const transcribeWithFactoryMock = vi.fn();
const encodeBlobToWavMock = vi.fn();
vi.mock('./voice/sttClient', () => ({
  transcribeWithFactory: (...args: unknown[]) => transcribeWithFactoryMock(...args),
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

describe('MicComposer', () => {
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
    transcribeWithFactoryMock.mockReset();
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
    vi.useRealTimers();
    if (originalMediaDevicesDescriptor) {
      Object.defineProperty(globalThis.navigator, 'mediaDevices', originalMediaDevicesDescriptor);
    } else {
      delete (globalThis.navigator as { mediaDevices?: MediaDevices }).mediaDevices;
    }
    vi.unstubAllGlobals();
  });

  it('renders the idle "Tap and speak" state', () => {
    render(<MicComposer disabled={false} onSubmit={vi.fn()} />);
    expect(screen.getByText('Tap and speak')).toBeInTheDocument();
  });

  it('shows a "Waiting" label when disabled', () => {
    render(<MicComposer disabled={true} onSubmit={vi.fn()} />);
    expect(screen.getByText(/waiting/i)).toBeInTheDocument();
  });

  it('does not start recording when disabled', () => {
    render(<MicComposer disabled={true} onSubmit={vi.fn()} />);
    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    expect(getUserMediaMock).not.toHaveBeenCalled();
  });

  it('starts recording on tap, then transcribes + submits on second tap', async () => {
    transcribeWithFactoryMock.mockResolvedValueOnce('hello world');
    const onSubmit = vi.fn();
    const onError = vi.fn();
    render(<MicComposer disabled={false} onSubmit={onSubmit} onError={onError} />);

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
    expect(transcribeWithFactoryMock).toHaveBeenCalledTimes(1);
  });

  it('forwards the language prop to transcribeCloud', async () => {
    transcribeWithFactoryMock.mockResolvedValueOnce('hi');
    render(<MicComposer disabled={false} onSubmit={vi.fn()} language="es" />);
    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /stop recording and send/i })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('button', { name: /stop recording and send/i }));
    await waitFor(() => expect(transcribeWithFactoryMock).toHaveBeenCalled());
    const opts = transcribeWithFactoryMock.mock.calls[0][1];
    expect(opts).toEqual({ language: 'es' });
  });

  it('surfaces a permission-denied error via onError for NotAllowedError', async () => {
    const err = Object.assign(new DOMException('', 'NotAllowedError'));
    getUserMediaMock.mockRejectedValueOnce(err);
    const onError = vi.fn();
    render(<MicComposer disabled={false} onSubmit={vi.fn()} onError={onError} />);
    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    await waitFor(() => expect(onError).toHaveBeenCalledWith(expect.stringMatching(/permission/i)));
  });

  it('surfaces a device-unavailable error for OverconstrainedError', async () => {
    const err = new DOMException('', 'OverconstrainedError');
    getUserMediaMock.mockRejectedValueOnce(err);
    const onError = vi.fn();
    render(<MicComposer disabled={false} onSubmit={vi.fn()} onError={onError} />);
    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    await waitFor(() =>
      expect(onError).toHaveBeenCalledWith(expect.stringMatching(/unavailable/i))
    );
  });

  it('surfaces an in-use error for NotReadableError', async () => {
    const err = new DOMException('', 'NotReadableError');
    getUserMediaMock.mockRejectedValueOnce(err);
    const onError = vi.fn();
    render(<MicComposer disabled={false} onSubmit={vi.fn()} onError={onError} />);
    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    await waitFor(() => expect(onError).toHaveBeenCalledWith(expect.stringMatching(/in use/i)));
  });

  it('surfaces a generic error for non-DOMException getUserMedia failures', async () => {
    getUserMediaMock.mockRejectedValueOnce(new Error('some other error'));
    const onError = vi.fn();
    render(<MicComposer disabled={false} onSubmit={vi.fn()} onError={onError} />);
    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    await waitFor(() =>
      expect(onError).toHaveBeenCalledWith(expect.stringMatching(/microphone error/i))
    );
  });

  it('surfaces a generic error for an unrecognized DOMException name', async () => {
    getUserMediaMock.mockRejectedValueOnce(new DOMException('boom', 'AbortError'));
    const onError = vi.fn();
    render(<MicComposer disabled={false} onSubmit={vi.fn()} onError={onError} />);
    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    await waitFor(() =>
      expect(onError).toHaveBeenCalledWith(expect.stringMatching(/microphone error/i))
    );
  });

  it('falls back to wav re-encode when the native attempt fails', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    // Native path: all 3 attempts (initial + 2 retries) fail, then WAV succeeds.
    transcribeWithFactoryMock
      .mockRejectedValueOnce(new Error('codec not accepted'))
      .mockRejectedValueOnce(new Error('codec not accepted'))
      .mockRejectedValueOnce(new Error('codec not accepted'))
      .mockResolvedValueOnce('after fallback');
    encodeBlobToWavMock.mockResolvedValueOnce(
      new Blob([new Uint8Array([0])], { type: 'audio/wav' })
    );
    const onSubmit = vi.fn();
    render(<MicComposer disabled={false} onSubmit={onSubmit} />);
    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /stop recording and send/i })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('button', { name: /stop recording and send/i }));
    await waitFor(() => expect(transcribeWithFactoryMock).toHaveBeenCalledTimes(1));
    await vi.advanceTimersByTimeAsync(500);
    await waitFor(() => expect(transcribeWithFactoryMock).toHaveBeenCalledTimes(2));
    await vi.advanceTimersByTimeAsync(1000);
    await waitFor(() => expect(onSubmit).toHaveBeenCalledWith('after fallback'));
    expect(encodeBlobToWavMock).toHaveBeenCalledTimes(1);
    // 3 native attempts + 1 WAV attempt
    expect(transcribeWithFactoryMock).toHaveBeenCalledTimes(4);
  });

  it('reports an error when transcription returns empty text', async () => {
    transcribeWithFactoryMock.mockResolvedValueOnce('');
    const onError = vi.fn();
    const onSubmit = vi.fn();
    render(<MicComposer disabled={false} onSubmit={onSubmit} onError={onError} />);
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
    render(<MicComposer disabled={false} onSubmit={vi.fn()} onError={onError} />);
    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    expect(onError).toHaveBeenCalledWith(expect.stringMatching(/not available/i));
  });

  // ── Spacebar shortcut (#1471) ────────────────────────────────────────────

  it('spacebar starts recording when idle and stops + submits on second press', async () => {
    transcribeWithFactoryMock.mockResolvedValueOnce('voice via space');
    const onSubmit = vi.fn();
    render(<MicComposer disabled={false} onSubmit={onSubmit} />);

    fireEvent.keyDown(window, { code: 'Space' });
    await waitFor(() => expect(getUserMediaMock).toHaveBeenCalled());
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /stop recording and send/i })).toBeInTheDocument()
    );

    fireEvent.keyDown(window, { code: 'Space' });
    await waitFor(() => expect(onSubmit).toHaveBeenCalledWith('voice via space'));
  });

  it('spacebar ignores key repeat so holding the key does not flap the recorder', () => {
    render(<MicComposer disabled={false} onSubmit={vi.fn()} />);
    fireEvent.keyDown(window, { code: 'Space', repeat: true });
    expect(getUserMediaMock).not.toHaveBeenCalled();
  });

  it('spacebar ignores modifier combinations so Shift-Space etc. stay free', () => {
    render(<MicComposer disabled={false} onSubmit={vi.fn()} />);
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
        <MicComposer disabled={false} onSubmit={vi.fn()} />
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
        <MicComposer disabled={false} onSubmit={vi.fn()} />
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
        <MicComposer disabled={false} onSubmit={vi.fn()} />
      </>
    );
    const ce = screen.getByTestId('ce');
    ce.focus();
    fireEvent.keyDown(ce, { code: 'Space' });
    expect(getUserMediaMock).not.toHaveBeenCalled();
  });

  it('spacebar is a no-op while the composer is disabled', () => {
    render(<MicComposer disabled={true} onSubmit={vi.fn()} />);
    fireEvent.keyDown(window, { code: 'Space' });
    expect(getUserMediaMock).not.toHaveBeenCalled();
  });

  it('removes the window keydown listener on unmount', () => {
    const removeSpy = vi.spyOn(window, 'removeEventListener');
    const { unmount } = render(<MicComposer disabled={false} onSubmit={vi.fn()} />);
    unmount();
    expect(removeSpy).toHaveBeenCalledWith('keydown', expect.any(Function));
    removeSpy.mockRestore();
  });

  // ── Device selector (showDeviceSelector) ─────────────────────────────────

  // ── Device selector: gear FAB + portaled menu (replaced <select> combobox) ──

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

    render(<MicComposer disabled={false} onSubmit={vi.fn()} showDeviceSelector />);

    await waitFor(() => expect(enumerateDevicesMock).toHaveBeenCalled());
    // The gear FAB is shown when devices.length > 1.
    const gearBtn = await screen.findByLabelText(/Microphone device/i);
    expect(gearBtn).toBeInTheDocument();
    // Open the menu to see device items.
    fireEvent.click(gearBtn);
    expect(await screen.findByRole('menuitemradio', { name: /Built-in Mic/i })).toBeInTheDocument();
    expect(screen.getByRole('menuitemradio', { name: /USB Headset/i })).toBeInTheDocument();
    // Video input must not appear.
    expect(screen.queryByRole('menuitemradio', { name: /Camera/i })).not.toBeInTheDocument();
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

    render(<MicComposer disabled={false} onSubmit={vi.fn()} />);

    await waitFor(() => {
      expect(screen.queryByLabelText(/Microphone device/i)).not.toBeInTheDocument();
      expect(enumerateDevicesMock).not.toHaveBeenCalled();
    });
  });

  it('does not show the gear FAB when only one device is available', async () => {
    const enumerateDevicesMock = vi
      .fn()
      .mockResolvedValue([{ kind: 'audioinput', deviceId: 'dev1', label: 'Built-in Mic' }]);
    Object.defineProperty(globalThis.navigator, 'mediaDevices', {
      value: { getUserMedia: getUserMediaMock, enumerateDevices: enumerateDevicesMock },
      configurable: true,
      writable: true,
    });

    render(<MicComposer disabled={false} onSubmit={vi.fn()} showDeviceSelector />);

    await waitFor(() => expect(enumerateDevicesMock).toHaveBeenCalled());
    // With only one device the gear FAB is not rendered at all.
    expect(screen.queryByLabelText(/Microphone device/i)).not.toBeInTheDocument();
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

    render(<MicComposer disabled={false} onSubmit={vi.fn()} showDeviceSelector />);

    // Gear FAB appears when there are >1 devices.
    const gearBtn = await screen.findByLabelText(/Microphone device/i);
    expect(gearBtn).toBeInTheDocument();
    fireEvent.click(gearBtn);
    await waitFor(() =>
      expect(screen.getByRole('menuitemradio', { name: /Microphone 1/i })).toBeInTheDocument()
    );
    expect(screen.getByRole('menuitemradio', { name: /Microphone 2/i })).toBeInTheDocument();
  });

  it('passes the selected deviceId as an exact constraint to getUserMedia', async () => {
    const enumerateDevicesMock = vi.fn().mockResolvedValue([
      { kind: 'audioinput', deviceId: 'dev1', label: 'Built-in Mic' },
      { kind: 'audioinput', deviceId: 'dev2', label: 'USB Headset' },
    ]);
    transcribeWithFactoryMock.mockResolvedValueOnce('hello');
    Object.defineProperty(globalThis.navigator, 'mediaDevices', {
      value: { getUserMedia: getUserMediaMock, enumerateDevices: enumerateDevicesMock },
      configurable: true,
      writable: true,
    });

    render(<MicComposer disabled={false} onSubmit={vi.fn()} showDeviceSelector />);

    // Open the gear menu and pick the second device.
    const gearBtn = await screen.findByLabelText(/Microphone device/i);
    fireEvent.click(gearBtn);
    const usbOption = await screen.findByRole('menuitemradio', { name: /USB Headset/i });
    fireEvent.click(usbOption);

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
    transcribeWithFactoryMock.mockResolvedValueOnce('ok');
    Object.defineProperty(globalThis.navigator, 'mediaDevices', {
      value: { getUserMedia: getUserMediaMock, enumerateDevices: enumerateDevicesMock },
      configurable: true,
      writable: true,
    });

    render(<MicComposer disabled={false} onSubmit={vi.fn()} showDeviceSelector />);

    // Mount enumerate ran — labels are blank placeholders; gear FAB visible.
    const gearBtn = await screen.findByLabelText(/Microphone device/i);
    expect(gearBtn).toBeInTheDocument();

    // Start recording → triggers the post-permission enumerate refresh.
    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /stop recording and send/i })).toBeInTheDocument()
    );
    // enumerateDevices should have been called again (post-permission refresh).
    await waitFor(() => expect(enumerateDevicesMock).toHaveBeenCalledTimes(2));

    // Stop recording so the gear button is re-enabled (disabled while recording).
    fireEvent.click(screen.getByRole('button', { name: /stop recording and send/i }));
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /start recording/i })).toBeInTheDocument()
    );

    // Open the menu to verify real labels are now shown.
    fireEvent.click(gearBtn);
    await waitFor(() =>
      expect(screen.getByRole('menuitemradio', { name: /Built-in Mic/i })).toBeInTheDocument()
    );
    expect(screen.getByRole('menuitemradio', { name: /USB Headset/i })).toBeInTheDocument();
  });

  // ── Recording timeout (#1206) ──────────────────────────────────────────────

  it('auto-stops recording after MAX_RECORDING_MS', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    transcribeWithFactoryMock.mockResolvedValueOnce('timeout transcript');
    const onSubmit = vi.fn();
    render(<MicComposer disabled={false} onSubmit={onSubmit} />);

    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    await waitFor(() => expect(getUserMediaMock).toHaveBeenCalled());
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /stop recording and send/i })).toBeInTheDocument()
    );

    // Advance past MAX_RECORDING_MS — the timeout should auto-stop.
    vi.advanceTimersByTime(MAX_RECORDING_MS + 100);
    await waitFor(() => expect(onSubmit).toHaveBeenCalledWith('timeout transcript'));
  });

  it('shows countdown label when remaining time <= 10s', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    transcribeWithFactoryMock.mockResolvedValueOnce('ok');
    render(<MicComposer disabled={false} onSubmit={vi.fn()} />);

    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    await waitFor(() => expect(getUserMediaMock).toHaveBeenCalled());
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /stop recording and send/i })).toBeInTheDocument()
    );

    // Initially shows "Tap to send" (not the countdown — remainingSecs=60 > 10).
    expect(screen.getByText('Tap to send')).toBeInTheDocument();

    // Advance 51 interval ticks (51s). The decrementing counter goes
    // from 60 → 9, which is <= 10 so the countdown label appears.
    vi.advanceTimersByTime(51_000);

    // The label pattern: "Tap to send (9s)" — match the parenthesized digit+s.
    await waitFor(() => expect(screen.getByText(/\d+s\)/)).toBeInTheDocument());
  });

  it('handles enumerateDevices throwing gracefully (no crash, selector hidden)', async () => {
    const enumerateDevicesMock = vi.fn().mockRejectedValue(new Error('NotAllowed'));
    Object.defineProperty(globalThis.navigator, 'mediaDevices', {
      value: { getUserMedia: getUserMediaMock, enumerateDevices: enumerateDevicesMock },
      configurable: true,
      writable: true,
    });

    render(<MicComposer disabled={false} onSubmit={vi.fn()} showDeviceSelector />);

    await waitFor(() => expect(enumerateDevicesMock).toHaveBeenCalled());
    // Selector requires >1 device; error yields 0 → selector stays hidden
    expect(screen.queryByRole('combobox', { name: /microphone device/i })).not.toBeInTheDocument();
    // Composer still functional
    expect(screen.getByText('Tap and speak')).toBeInTheDocument();
  });

  // ── STT retry (#1206) ──────────────────────────────────────────────────────

  it('retries transient STT failures before falling back to WAV', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    // Native path: fail twice (transient), then succeed
    transcribeWithFactoryMock
      .mockRejectedValueOnce(new Error('network timeout'))
      .mockRejectedValueOnce(new Error('network timeout'))
      .mockResolvedValueOnce('retry success');
    const onSubmit = vi.fn();
    render(<MicComposer disabled={false} onSubmit={onSubmit} />);

    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    await waitFor(() => expect(getUserMediaMock).toHaveBeenCalled());
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /stop recording and send/i })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('button', { name: /stop recording and send/i }));
    await waitFor(() => expect(transcribeWithFactoryMock).toHaveBeenCalledTimes(1));
    await vi.advanceTimersByTimeAsync(500);
    await waitFor(() => expect(transcribeWithFactoryMock).toHaveBeenCalledTimes(2));
    await vi.advanceTimersByTimeAsync(1000);
    await waitFor(() => expect(onSubmit).toHaveBeenCalledWith('retry success'));
    // Should have called transcribe 3 times (initial + 2 retries), no WAV fallback
    expect(transcribeWithFactoryMock).toHaveBeenCalledTimes(3);
    expect(encodeBlobToWavMock).not.toHaveBeenCalled();
  });

  it('falls back to WAV after exhausting native retries', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    // Native path: all 3 attempts fail, then WAV path succeeds
    transcribeWithFactoryMock
      .mockRejectedValueOnce(new Error('server error'))
      .mockRejectedValueOnce(new Error('server error'))
      .mockRejectedValueOnce(new Error('server error'))
      .mockResolvedValueOnce('wav retry ok');
    encodeBlobToWavMock.mockResolvedValueOnce(
      new Blob([new Uint8Array([0])], { type: 'audio/wav' })
    );
    const onSubmit = vi.fn();
    render(<MicComposer disabled={false} onSubmit={onSubmit} />);

    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    await waitFor(() => expect(getUserMediaMock).toHaveBeenCalled());
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /stop recording and send/i })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('button', { name: /stop recording and send/i }));
    await waitFor(() => expect(transcribeWithFactoryMock).toHaveBeenCalledTimes(1));
    await vi.advanceTimersByTimeAsync(500);
    await waitFor(() => expect(transcribeWithFactoryMock).toHaveBeenCalledTimes(2));
    await vi.advanceTimersByTimeAsync(1000);
    await waitFor(() => expect(onSubmit).toHaveBeenCalledWith('wav retry ok'));
    // 3 native attempts + 1 WAV attempt
    expect(transcribeWithFactoryMock).toHaveBeenCalledTimes(4);
    expect(encodeBlobToWavMock).toHaveBeenCalledTimes(1);
  });

  it('does not continue retrying after unmount during STT backoff', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    transcribeWithFactoryMock
      .mockRejectedValueOnce(new Error('network timeout'))
      .mockResolvedValueOnce('late transcript');
    const onSubmit = vi.fn();
    const onError = vi.fn();
    const view = render(<MicComposer disabled={false} onSubmit={onSubmit} onError={onError} />);

    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    await waitFor(() => expect(getUserMediaMock).toHaveBeenCalled());
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /stop recording and send/i })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('button', { name: /stop recording and send/i }));
    await waitFor(() => expect(transcribeWithFactoryMock).toHaveBeenCalledTimes(1));

    view.unmount();
    await vi.advanceTimersByTimeAsync(500);

    expect(transcribeWithFactoryMock).toHaveBeenCalledTimes(1);
    expect(onSubmit).not.toHaveBeenCalled();
    expect(onError).not.toHaveBeenCalled();
  });

  it('does not retry permanent errors (stale sidecar)', async () => {
    transcribeWithFactoryMock.mockRejectedValueOnce(
      new Error(
        'Voice transcription is unavailable in this build. Restart the OpenHuman desktop app to pick up the latest core sidecar.'
      )
    );
    const onError = vi.fn();
    const onSubmit = vi.fn();
    render(<MicComposer disabled={false} onSubmit={onSubmit} onError={onError} />);

    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    await waitFor(() => expect(getUserMediaMock).toHaveBeenCalled());
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /stop recording and send/i })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('button', { name: /stop recording and send/i }));
    await waitFor(() => expect(onError).toHaveBeenCalledWith(expect.stringMatching(/failed/i)));
    // Only 1 attempt — no retries for permanent errors
    expect(transcribeWithFactoryMock).toHaveBeenCalledTimes(1);
    expect(onSubmit).not.toHaveBeenCalled();
  });

  // ── Low-confidence transcript detection (#1206) ────────────────────────────

  it('rejects a single-character transcript as low confidence', async () => {
    transcribeWithFactoryMock.mockResolvedValueOnce('I');
    const onError = vi.fn();
    const onSubmit = vi.fn();
    render(<MicComposer disabled={false} onSubmit={onSubmit} onError={onError} />);

    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    await waitFor(() => expect(getUserMediaMock).toHaveBeenCalled());
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /stop recording and send/i })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('button', { name: /stop recording and send/i }));
    await waitFor(() =>
      expect(onError).toHaveBeenCalledWith(expect.stringMatching(/could not understand/i))
    );
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it('rejects a repeated-character transcript as low confidence', async () => {
    transcribeWithFactoryMock.mockResolvedValueOnce('aaa');
    const onError = vi.fn();
    const onSubmit = vi.fn();
    render(<MicComposer disabled={false} onSubmit={onSubmit} onError={onError} />);

    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    await waitFor(() => expect(getUserMediaMock).toHaveBeenCalled());
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /stop recording and send/i })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('button', { name: /stop recording and send/i }));
    await waitFor(() =>
      expect(onError).toHaveBeenCalledWith(expect.stringMatching(/could not understand/i))
    );
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it('accepts a normal transcript and submits it', async () => {
    transcribeWithFactoryMock.mockResolvedValueOnce('Hello, how are you?');
    const onSubmit = vi.fn();
    render(<MicComposer disabled={false} onSubmit={onSubmit} />);

    fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
    await waitFor(() => expect(getUserMediaMock).toHaveBeenCalled());
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /stop recording and send/i })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('button', { name: /stop recording and send/i }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledWith('Hello, how are you?'));
  });

  it('retry counter is monotone across native + WAV paths (no double-count)', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      // 3 native failures + 2 WAV failures + 1 WAV success = 6 calls total.
      //
      // The double-count bug only manifests on the SECOND WAV retry callback:
      //   fixed:  snapshot nativeRetries=2 → handleRetry(2+2)=4 → "4 of 4"
      //   buggy:  read mutable cumulativeRetries=3 → handleRetry(3+2)=5 → "5 of 4"
      //
      // By the time WAV attempt 2 (call 6, the success) starts executing, React
      // has already flushed the setRetryCount update from WAV attempt 1's retry
      // callback (which fired synchronously before the 500ms backoff sleep that
      // immediately preceded call 6). We read the rendered <span> text inside
      // that mock to assert the actual value the component shows, not a literal.
      let retryLabelAtSuccessCall = '';
      transcribeWithFactoryMock
        .mockRejectedValueOnce(new Error('native 0'))
        .mockRejectedValueOnce(new Error('native 1'))
        .mockRejectedValueOnce(new Error('native 2'))
        .mockRejectedValueOnce(new Error('wav 0'))
        .mockRejectedValueOnce(new Error('wav 1'))
        .mockImplementationOnce(async () => {
          // WAV attempt 2 — the success call. At this point setRetryCount was
          // called synchronously before the 500ms backoff that preceded this call.
          // React flushed during that backoff (fake-timer advancement is act-wrapped),
          // so the DOM reflects the actual retryCount value: 4 (fixed) or 5 (buggy).
          const el = document.querySelector('span.text-xs');
          retryLabelAtSuccessCall = el?.textContent ?? '';
          return 'cross-path ok';
        });
      encodeBlobToWavMock.mockResolvedValueOnce(
        new Blob([new Uint8Array([0])], { type: 'audio/wav' })
      );
      const onSubmit = vi.fn();
      render(<MicComposer disabled={false} onSubmit={onSubmit} />);

      fireEvent.click(screen.getByRole('button', { name: /start recording/i }));
      await waitFor(() => expect(getUserMediaMock).toHaveBeenCalled());
      await waitFor(() =>
        expect(screen.getByRole('button', { name: /stop recording and send/i })).toBeInTheDocument()
      );
      fireEvent.click(screen.getByRole('button', { name: /stop recording and send/i }));

      // Drive all 6 calls to completion by advancing through backoffs.
      // native: 500ms + 1000ms, WAV: 500ms + 1000ms.
      await vi.advanceTimersByTimeAsync(500);
      await vi.advanceTimersByTimeAsync(1000);
      await vi.advanceTimersByTimeAsync(500);
      await vi.advanceTimersByTimeAsync(1000);

      await waitFor(() => expect(onSubmit).toHaveBeenCalledWith('cross-path ok'));
      expect(transcribeWithFactoryMock).toHaveBeenCalledTimes(6);

      // The label seen inside call 6 must read "Retrying... (4 of 4)", not
      // "Retrying... (5 of 4)". If the snapshot fix is absent, cumulativeRetries
      // would be mutated to 3 after WAV retry 0, then 3+2=5 on WAV retry 1.
      expect(retryLabelAtSuccessCall).toMatch(/retrying.*4 of 4/i);
    } finally {
      vi.useRealTimers();
    }
  });
});

// ── isLowConfidenceTranscript unit tests ──────────────────────────────────

describe('isLowConfidenceTranscript', () => {
  it('flags empty strings', () => {
    expect(isLowConfidenceTranscript('')).toBe(true);
    expect(isLowConfidenceTranscript('  ')).toBe(true);
  });

  it('flags single characters', () => {
    expect(isLowConfidenceTranscript('I')).toBe(true);
    expect(isLowConfidenceTranscript('A')).toBe(true);
  });

  it('flags repeated single characters', () => {
    expect(isLowConfidenceTranscript('aaa')).toBe(true);
    expect(isLowConfidenceTranscript('mmm')).toBe(true);
    expect(isLowConfidenceTranscript('...')).toBe(true);
  });

  it('flags punctuation-only strings', () => {
    expect(isLowConfidenceTranscript('...')).toBe(true);
    expect(isLowConfidenceTranscript('!?')).toBe(true);
    expect(isLowConfidenceTranscript('---')).toBe(true);
  });

  it('accepts normal text', () => {
    expect(isLowConfidenceTranscript('Hello')).toBe(false);
    expect(isLowConfidenceTranscript('Hi there')).toBe(false);
    expect(isLowConfidenceTranscript('OK')).toBe(false);
  });

  it('accepts non-Latin scripts', () => {
    expect(isLowConfidenceTranscript('Hola')).toBe(false);
    expect(isLowConfidenceTranscript('Привет')).toBe(false);
    expect(isLowConfidenceTranscript('こんにちは')).toBe(false);
    expect(isLowConfidenceTranscript('안녕하세요')).toBe(false);
    expect(isLowConfidenceTranscript('مرحبا')).toBe(false);
    expect(isLowConfidenceTranscript('नमस्ते')).toBe(false);
    expect(isLowConfidenceTranscript('বাংলা')).toBe(false);
  });
});

describe('STT_MAX_RETRIES export', () => {
  it('is a positive integer', () => {
    expect(Number.isInteger(STT_MAX_RETRIES)).toBe(true);
    expect(STT_MAX_RETRIES).toBeGreaterThan(0);
  });
});
