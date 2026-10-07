import { ComposerTextBridge } from '@/features/conversations/components/AssistantUiChat';
import en from '@/lib/i18n/en';
import fr from '@/lib/i18n/fr';
import { I18nProvider } from '@/lib/i18n/I18nContext';
import {
  ComposerDictationContext,
  type ComposerDictationState,
} from '@/providers/ComposerDictationContext';
import {
  createOpenHumanDictationAdapter,
  type OpenHumanDictationAdapter,
} from '@/providers/dictationAdapter';
import {
  type AppendMessage,
  AssistantRuntimeProvider,
  type ThreadMessageLike,
  useExternalStoreRuntime,
} from '@assistant-ui/react';
import { configureStore } from '@reduxjs/toolkit';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { useState, useSyncExternalStore } from 'react';
import { Provider } from 'react-redux';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { ComposerDictationControls, ComposerDictationStatus } from './composer-dictation';
import { Thread } from './thread';

const transcribe = vi.fn();
const encodeWav = vi.fn();
vi.mock('@/features/human/voice/sttClient', async importOriginal => ({
  ...(await importOriginal<typeof import('@/features/human/voice/sttClient')>()),
  transcribeWithFactory: (...args: unknown[]) => transcribe(...args),
}));
vi.mock('@/features/human/voice/wavEncoder', () => ({
  encodeBlobToWav: (...args: unknown[]) => encodeWav(...args),
}));

const NO_MESSAGES: ThreadMessageLike[] = [];

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(res => {
    resolve = res;
  });
  return { promise, resolve };
}

function Harness({
  adapter,
  onNew,
  onEscape,
  onSwitchToMicCloud,
  isRunning = false,
}: {
  adapter: OpenHumanDictationAdapter;
  onNew: (message: AppendMessage) => Promise<void>;
  onEscape?: () => void;
  onSwitchToMicCloud?: () => void;
  isRunning?: boolean;
}) {
  const [draft, setDraft] = useState('');
  const [voiceMode, setVoiceMode] = useState(false);
  const snapshot = useSyncExternalStore(adapter.subscribe, adapter.getSnapshot);
  const runtime = useExternalStoreRuntime({
    messages: NO_MESSAGES,
    isRunning,
    convertMessage: message => message,
    onNew,
    onCancel: async () => {},
    adapters: { dictation: adapter },
  });
  return (
    <AssistantRuntimeProvider runtime={runtime}>
      <ComposerDictationContext.Provider
        value={{ adapter, status: snapshot.phase, error: snapshot.error, cancel: adapter.cancel }}>
        <ComposerTextBridge value={draft} onChange={setDraft} />
        <div data-testid="host-draft">{draft}</div>
        {voiceMode ? (
          <div>Voice composer</div>
        ) : (
          <Thread
            model={null}
            onEscape={onEscape}
            components={{
              ...(onSwitchToMicCloud
                ? {
                    onSwitchToMicCloud: () => {
                      onSwitchToMicCloud();
                      setVoiceMode(true);
                    },
                  }
                : {}),
            }}
          />
        )}
      </ComposerDictationContext.Provider>
    </AssistantRuntimeProvider>
  );
}

describe('dictation on the editable Thread composer', () => {
  let adapter: OpenHumanDictationAdapter;
  let track: EventTarget & { stop: ReturnType<typeof vi.fn>; readyState: string };
  let getUserMedia: ReturnType<typeof vi.fn>;
  let originalMediaDevices: PropertyDescriptor | undefined;
  let stopRecorder: ReturnType<typeof vi.fn<() => void>>;

  beforeEach(() => {
    transcribe.mockReset().mockResolvedValue('spoken addition');
    encodeWav.mockReset();
    track = Object.assign(new EventTarget(), { stop: vi.fn(), readyState: 'live' });
    getUserMedia = vi.fn().mockResolvedValue({ getTracks: () => [track] });
    originalMediaDevices = Object.getOwnPropertyDescriptor(navigator, 'mediaDevices');
    Object.defineProperty(navigator, 'mediaDevices', {
      configurable: true,
      value: { getUserMedia },
    });
    class Recorder {
      state: RecordingState = 'inactive';
      mimeType = 'audio/webm';
      ondataavailable: ((event: BlobEvent) => void) | null = null;
      onstop: (() => void) | null = null;
      onerror: (() => void) | null = null;
      constructor() {
        stopRecorder = vi.fn(() => {
          this.state = 'inactive';
          this.ondataavailable?.({
            data: new Blob(['audio'], { type: this.mimeType }),
          } as BlobEvent);
          this.onstop?.();
        });
      }
      start() {
        this.state = 'recording';
      }
      stop() {
        stopRecorder();
      }
      static isTypeSupported() {
        return false;
      }
    }
    vi.stubGlobal('MediaRecorder', Recorder);
    adapter = createOpenHumanDictationAdapter();
  });

  afterEach(() => {
    adapter.dispose();
    if (originalMediaDevices) {
      Object.defineProperty(navigator, 'mediaDevices', originalMediaDevices);
    } else {
      delete (navigator as { mediaDevices?: MediaDevices }).mediaDevices;
    }
    vi.unstubAllGlobals();
  });

  function mount(
    options: { onEscape?: () => void; onSwitchToMicCloud?: () => void; isRunning?: boolean } = {}
  ) {
    const onNew = vi.fn().mockResolvedValue(undefined);
    const view = render(<Harness adapter={adapter} onNew={onNew} {...options} />);
    return { ...view, onNew };
  }

  async function start() {
    await typeDraft('Draft');
    fireEvent.click(screen.getByRole('button', { name: 'Dictate' }));
    await screen.findByRole('button', { name: 'Finish dictation' });
    expect(screen.getByRole('status')).toHaveTextContent(/recording/i);
  }

  async function typeDraft(text: string) {
    const input = screen.getByTestId('chat-message-input');
    expect(input).toHaveAttribute('contenteditable', 'true');
    input.textContent = text;
    fireEvent.input(input);
    await waitFor(() => expect(screen.getByTestId('host-draft')).toHaveTextContent(text));
    return input;
  }

  it('keeps the typed draft editable while recording and appends final text only after Finish', async () => {
    const result = deferred<string>();
    transcribe.mockReturnValue(result.promise);
    const { onNew } = mount();
    await start();
    await typeDraft('Draft edited while recording');
    expect(transcribe).not.toHaveBeenCalled();
    expect(onNew).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole('button', { name: 'Finish dictation' }));
    await waitFor(() => expect(screen.getByRole('status')).toHaveTextContent(/transcribing/i));
    expect(screen.getByTestId('host-draft')).toHaveTextContent('Draft edited while recording');
    await typeDraft('Draft edited while transcribing');
    await act(async () => {
      result.resolve('spoken addition');
    });

    await waitFor(() =>
      expect(screen.getByTestId('host-draft')).toHaveTextContent(
        'Draft edited while transcribing spoken addition'
      )
    );
    expect(screen.getByTestId('chat-message-input')).toHaveTextContent(
      'Draft edited while transcribing spoken addition'
    );
    expect(onNew).not.toHaveBeenCalled();
    expect(track.stop).toHaveBeenCalledOnce();
    expect(screen.queryByRole('status')).toBeNull();
    expect(screen.getByRole('button', { name: 'Dictate' })).toBeEnabled();
  });

  it('waits for WAV conversion and appends the retry result once to the edited draft', async () => {
    const conversion = deferred<Blob>();
    const retry = deferred<string>();
    transcribe
      .mockRejectedValueOnce(new Error('native container rejected'))
      .mockReturnValueOnce(retry.promise);
    encodeWav.mockReturnValue(conversion.promise);
    const { onNew } = mount();
    await start();
    fireEvent.click(screen.getByRole('button', { name: 'Finish dictation' }));

    await waitFor(() => expect(encodeWav).toHaveBeenCalledOnce());
    const native = transcribe.mock.calls[0][0] as Blob;
    expect(native.type).toBe('audio/webm');
    expect(encodeWav).toHaveBeenCalledWith(native);
    expect(transcribe).toHaveBeenCalledOnce();
    expect(track.stop).toHaveBeenCalledOnce();
    await waitFor(() => expect(screen.getByRole('status')).toHaveTextContent(/transcribing/i));
    await typeDraft('Edited during conversion');

    const wav = new Blob(['PCM fixture'], { type: 'audio/wav' });
    await act(async () => conversion.resolve(wav));
    await waitFor(() => expect(transcribe).toHaveBeenNthCalledWith(2, wav));
    expect(screen.getByTestId('host-draft')).toHaveTextContent('Edited during conversion');
    await typeDraft('Edited during retry');
    await act(async () => retry.resolve('fallback words'));

    await waitFor(() =>
      expect(screen.getByTestId('host-draft')).toHaveTextContent(
        'Edited during retry fallback words'
      )
    );
    expect(screen.getByTestId('chat-message-input').textContent).toBe(
      'Edited during retry fallback words'
    );
    expect(screen.queryByRole('status')).toBeNull();
    expect(screen.getByRole('button', { name: 'Dictate' })).toBeEnabled();
    expect(transcribe).toHaveBeenCalledTimes(2);
    expect(encodeWav).toHaveBeenCalledOnce();
    expect(onNew).not.toHaveBeenCalled();
  });

  it('sends the edited dictated text only after the user explicitly sends it', async () => {
    const { onNew } = mount();
    await start();
    fireEvent.click(screen.getByRole('button', { name: 'Finish dictation' }));
    await waitFor(() =>
      expect(screen.getByTestId('host-draft')).toHaveTextContent('Draft spoken addition')
    );
    expect(onNew).not.toHaveBeenCalled();
    await typeDraft('Final reviewed message');
    fireEvent.click(screen.getByRole('button', { name: 'Send message' }));
    await waitFor(() => expect(onNew).toHaveBeenCalledOnce());
    expect(onNew.mock.calls[0][0].content).toEqual([
      { type: 'text', text: 'Final reviewed message' },
    ]);
    await waitFor(() => expect(screen.getByTestId('host-draft')).toBeEmptyDOMElement());
  });

  it('Discard during STT preserves edits and ignores the late transcript', async () => {
    const result = deferred<string>();
    transcribe.mockReturnValue(result.promise);
    const { onNew } = mount();
    await start();
    await typeDraft('Keep this draft');
    fireEvent.click(screen.getByRole('button', { name: 'Finish dictation' }));
    await waitFor(() => expect(screen.getByRole('status')).toHaveTextContent(/transcribing/i));
    fireEvent.click(screen.getByRole('button', { name: 'Discard dictation' }));
    expect(screen.queryByRole('status')).toBeNull();
    expect(screen.getByRole('button', { name: 'Dictate' })).toBeEnabled();

    await act(async () => {
      result.resolve('discarded words');
    });
    expect(screen.getByTestId('host-draft')).toHaveTextContent('Keep this draft');
    expect(screen.getByTestId('chat-message-input')).toHaveTextContent('Keep this draft');
    expect(onNew).not.toHaveBeenCalled();
  });

  it('Escape cancels active dictation before the host Escape handler runs', async () => {
    const onEscape = vi.fn();
    const { onNew } = mount({ onEscape });
    await start();
    const input = await typeDraft('Preserved edit');
    fireEvent.keyDown(input, { key: 'Escape' });
    expect(adapter.getSnapshot().phase).toBe('idle');
    expect(track.stop).toHaveBeenCalledOnce();
    expect(onEscape).not.toHaveBeenCalled();
    expect(transcribe).not.toHaveBeenCalled();
    expect(screen.getByTestId('host-draft')).toHaveTextContent('Preserved edit');
    expect(onNew).not.toHaveBeenCalled();
    // Once dictation is idle, the ordinary host Escape behavior remains usable.
    fireEvent.keyDown(input, { key: 'Escape' });
    expect(onEscape).toHaveBeenCalledOnce();
  });

  it('unmount releases the microphone without transcribing or sending', async () => {
    const { unmount, onNew } = mount();
    await start();
    unmount();
    expect(adapter.getSnapshot().phase).toBe('idle');
    expect(track.stop).toHaveBeenCalledOnce();
    expect(transcribe).not.toHaveBeenCalled();
    expect(onNew).not.toHaveBeenCalled();
  });

  it('unmount during STT discards its late result', async () => {
    const result = deferred<string>();
    transcribe.mockReturnValue(result.promise);
    const { unmount, onNew } = mount();
    await start();
    fireEvent.click(screen.getByRole('button', { name: 'Finish dictation' }));
    unmount();
    await act(async () => {
      result.resolve('after unmount');
    });
    expect(adapter.getSnapshot().phase).toBe('idle');
    expect(onNew).not.toHaveBeenCalled();
    expect(track.stop).toHaveBeenCalledOnce();
  });

  it('Discard while permission is pending closes the session and releases a late grant', async () => {
    const permission = deferred<MediaStream>();
    getUserMedia.mockReturnValue(permission.promise);
    const { onNew } = mount();
    await typeDraft('Draft');
    fireEvent.click(screen.getByRole('button', { name: 'Dictate' }));
    await screen.findByText('Starting microphone...');
    fireEvent.click(screen.getByRole('button', { name: 'Discard dictation' }));
    await act(async () => {
      permission.resolve({ getTracks: () => [track] } as unknown as MediaStream);
    });
    expect(track.stop).toHaveBeenCalledOnce();
    expect(transcribe).not.toHaveBeenCalled();
    expect(screen.getByTestId('host-draft')).toHaveTextContent('Draft');
    expect(onNew).not.toHaveBeenCalled();
  });

  it('renders a translated permission error and keeps the draft', async () => {
    getUserMedia.mockRejectedValue(new DOMException('sensitive device detail', 'NotAllowedError'));
    mount();
    await typeDraft('Draft');
    fireEvent.click(screen.getByRole('button', { name: 'Dictate' }));
    expect(await screen.findByRole('alert')).toHaveTextContent(/microphone permission denied/i);
    expect(screen.getByRole('alert')).not.toHaveTextContent('sensitive');
    expect(screen.getByTestId('host-draft')).toHaveTextContent('Draft');
    expect(screen.getByRole('button', { name: 'Dictate' })).toBeEnabled();
  });

  it('does not start capture during an active agent turn', () => {
    mount({ isRunning: true });
    const button = screen.getByRole('button', { name: 'Dictate' });
    expect(button).toBeDisabled();
    fireEvent.click(button);
    expect(getUserMedia).not.toHaveBeenCalled();
  });

  it.each(['recording', 'transcribing'] as const)(
    'switching to Voice mode cancels %s before replacing the text composer',
    async phase => {
      const result = deferred<string>();
      transcribe.mockReturnValue(result.promise);
      const onSwitchToMicCloud = vi.fn(() => adapter.getSnapshot().phase);
      const { onNew } = mount({ onSwitchToMicCloud });
      await start();
      await typeDraft('Preserved voice mode draft');
      if (phase === 'transcribing') {
        fireEvent.click(screen.getByRole('button', { name: 'Finish dictation' }));
        await waitFor(() => expect(screen.getByRole('status')).toHaveTextContent(/transcribing/i));
      }

      fireEvent.click(screen.getByRole('button', { name: 'Voice mode' }));
      expect(onSwitchToMicCloud).toHaveBeenCalledOnce();
      expect(onSwitchToMicCloud.mock.results[0].value).toBe('idle');
      expect(track.stop).toHaveBeenCalledOnce();
      expect(screen.getByText('Voice composer')).toBeInTheDocument();
      expect(screen.queryByRole('button', { name: 'Discard dictation' })).toBeNull();
      await act(async () => {
        result.resolve('text from the old mode');
      });
      expect(screen.getByTestId('host-draft')).toHaveTextContent('Preserved voice mode draft');
      expect(onNew).not.toHaveBeenCalled();
      if (phase === 'recording') expect(transcribe).not.toHaveBeenCalled();
    }
  );
});

const ERROR_CASES = [
  ['stt-unavailable', 'composer.dictationUnavailable'],
  ['voice-status-failed', 'composer.dictationStatusFailed'],
  ['microphone-unavailable', 'mic.unavailable'],
  ['permission-denied', 'mic.permissionDenied'],
  ['device-unavailable', 'mic.deviceUnavailable'],
  ['device-in-use', 'mic.deviceInUse'],
  ['recorder-failed', 'mic.failedToStartRecorder'],
  ['no-audio', 'mic.noAudioCaptured'],
  ['no-speech', 'mic.noSpeechDetected'],
  ['transcription-failed', 'composer.dictationFailed'],
  ['timed-out', 'composer.dictationTimedOut'],
  ['voice-unavailable', 'mic.voiceNotCompiled'],
] as const;

function StatusHarness({
  status = 'idle',
  error = null,
}: {
  status?: ComposerDictationState['status'];
  error?: ComposerDictationState['error'];
}) {
  return (
    <ComposerDictationContext.Provider
      value={{ adapter: undefined, status, error, cancel: () => {} }}>
      <ComposerDictationStatus />
    </ComposerDictationContext.Provider>
  );
}

function ControlsHarness({ dictation }: { dictation: ComposerDictationState | null }) {
  const runtime = useExternalStoreRuntime({
    messages: NO_MESSAGES,
    convertMessage: message => message,
    onNew: async () => {},
    adapters: { dictation: dictation?.adapter },
  });
  return (
    <AssistantRuntimeProvider runtime={runtime}>
      <ComposerDictationContext.Provider value={dictation}>
        <ComposerDictationControls />
        <ComposerDictationStatus />
      </ComposerDictationContext.Provider>
    </AssistantRuntimeProvider>
  );
}

describe('localized dictation status and capability visibility', () => {
  describe.each([
    ['English', en],
    ['French', fr],
  ] as const)('%s errors', (language, translations) => {
    it.each(ERROR_CASES)('renders the localized alert for %s', (error, key) => {
      const store = configureStore({
        reducer: { locale: () => ({ current: language === 'French' ? 'fr' : 'en' }) },
      });
      render(
        <Provider store={store}>
          <I18nProvider>
            <StatusHarness error={error} status="transcribing" />
          </I18nProvider>
        </Provider>
      );
      expect(screen.getByRole('alert')).toHaveTextContent(translations[key]);
      expect(screen.queryByRole('status')).toBeNull();
    });
  });

  it.each([
    ['starting', 'composer.dictationStarting'],
    ['recording', 'composer.dictationRecording'],
    ['transcribing', 'mic.transcribing'],
  ] as const)('announces the %s phase', (status, key) => {
    render(<StatusHarness status={status} />);
    expect(screen.getByRole('status')).toHaveTextContent(en[key]);
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it.each(['idle', 'checking', 'unavailable'] as const)(
    'renders no active status in the %s state',
    status => {
      const { container } = render(<StatusHarness status={status} />);
      expect(container).toBeEmptyDOMElement();
    }
  );

  it('renders no controls or status without a dictation context', () => {
    const { container } = render(<ControlsHarness dictation={null} />);
    expect(container).toBeEmptyDOMElement();
  });

  it('renders no dictation controls without an available adapter', () => {
    const { container } = render(
      <ControlsHarness
        dictation={{ adapter: undefined, status: 'unavailable', error: null, cancel: () => {} }}
      />
    );
    expect(container).toBeEmptyDOMElement();
    expect(screen.queryByRole('button', { name: 'Dictate' })).toBeNull();
  });
});
