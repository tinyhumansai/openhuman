import debug from 'debug';
import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

import { useT } from '../../lib/i18n/I18nContext';
import { transcribeWithFactory } from './voice/sttClient';
import { encodeBlobToWav } from './voice/wavEncoder';

/** Minimal descriptor for an audio input device. */
interface AudioInputDevice {
  deviceId: string;
  label: string;
}

const composerLog = debug('human:mic-composer');

/**
 * Maximum recording duration in milliseconds. Auto-stops the recorder when
 * reached so accidental or forgotten recordings don't run indefinitely.
 * 60 seconds matches the practical ceiling for single-utterance STT accuracy.
 * Exported for test assertions.
 */
export const MAX_RECORDING_MS = 60_000;

/** Maximum number of retries for transient STT failures per encoding path. */
const STT_MAX_RETRIES = 2;

/** Base delay for exponential backoff (ms). Delay = base * 2^attempt. */
const STT_RETRY_BASE_MS = 500;

/**
 * Errors that indicate a permanent failure — retrying won't help.
 * Matched case-insensitively against the error message.
 */
const PERMANENT_ERROR_PATTERNS = [
  'unknown method', // stale sidecar
  'audio blob is empty',
  'unavailable in this build',
];

class TranscriptionCancelledError extends Error {
  constructor() {
    super('transcription cancelled');
    this.name = 'TranscriptionCancelledError';
  }
}

function isTranscriptionCancelledError(err: unknown): err is TranscriptionCancelledError {
  return err instanceof TranscriptionCancelledError;
}

function isTransientError(err: unknown): boolean {
  const msg = (err instanceof Error ? err.message : String(err)).toLowerCase();
  return !PERMANENT_ERROR_PATTERNS.some(p => msg.includes(p));
}

/**
 * Heuristic check for low-confidence transcripts. The backend doesn't expose
 * a confidence score, so we detect likely bad results from text patterns:
 * - Single character (often a stray noise → "I" or "A")
 * - Repeated single character ("aaa", "mmm")
 * - Only punctuation / whitespace
 * Exported for testing.
 */
export function isLowConfidenceTranscript(text: string): boolean {
  const trimmed = text.trim();
  if (trimmed.length === 0) return true;
  if (trimmed.length === 1) return true;
  // All same character repeated: "aaa", "mmm", "..."
  if (/^(.)\1+$/i.test(trimmed)) return true;
  // Only punctuation, whitespace, or symbols — no actual words
  if (!/\p{L}/u.test(trimmed)) return true;
  return false;
}

/** MIME types MediaRecorder will be asked to use, in priority order.
 *
 *  AAC-in-MP4 is preferred because the hosted STT upstream (GMI Whisper)
 *  rejected Opus-in-WebM with "Invalid JSON payload" — AAC is far more
 *  broadly accepted by OpenAI-compatible audio endpoints. We fall through
 *  to WebM/Opus on Chromium builds that haven't shipped MP4 recording, then
 *  to whatever the browser picks by default. */
const PREFERRED_MIMES = ['audio/mp4;codecs=mp4a.40.2', 'audio/mp4', 'audio/webm;codecs=opus'];

function pickRecorderMime(): string {
  if (typeof MediaRecorder === 'undefined') return '';
  for (const mime of PREFERRED_MIMES) {
    if (MediaRecorder.isTypeSupported(mime)) return mime;
  }
  return '';
}

export interface MicComposerProps {
  /** Disabled while a turn is in flight or the welcome message is pending. */
  disabled: boolean;
  /** Receives the transcribed text — same callback the textarea send uses. */
  onSubmit: (text: string) => Promise<void> | void;
  /** Surfaced when the mic flow fails so the parent can show a banner. */
  onError?: (message: string) => void;
  /** ISO 639-1 language hint forwarded to Scribe. Defaults to `'en'` —
   *  passing a hint is meaningfully more accurate than auto-detect on
   *  short utterances. Set to empty string to let Scribe auto-detect. */
  language?: string;
  /** Show a microphone device selector beneath the button. Defaults to false. */
  showDeviceSelector?: boolean;
  /** When provided, renders a keyboard FAB next to the gear that switches the
   *  surrounding composer back to text input. */
  onSwitchToText?: () => void;
}

type RecordingState = 'idle' | 'recording' | 'transcribing';

/**
 * Exported so tests can assert on the retry ceiling without duplicating the
 * constant.
 */
export { STT_MAX_RETRIES };

/**
 * Tap-to-toggle mic composer for the mascot page. Captures audio via the
 * browser's `MediaRecorder`, hands the resulting Blob to the factory-
 * dispatched STT RPC (`openhuman.voice_stt_dispatch`), then forwards the
 * transcript through `onSubmit` so it joins the agent's normal send pipeline.
 *
 * The provider (cloud vs local Whisper) is resolved server-side from
 * `config.local_ai.stt_provider`, so the renderer doesn't have to know
 * which backend ran — it only sees `{ text, provider }`.
 *
 * Single button, single decision: tap once to start recording, tap again to
 * stop and send. No textarea — that's the whole point of the mascot tab.
 */
export function MicComposer({
  disabled,
  onSubmit,
  onError,
  language = 'en',
  showDeviceSelector = false,
  onSwitchToText,
}: MicComposerProps) {
  const { t } = useT();
  const [state, setState] = useState<RecordingState>('idle');
  // Tracks how many STT retry attempts have been made for the current
  // recording, so the label can change from "Transcribing…" to
  // "Retrying… (1 of 2)" while the user waits.
  const [retryCount, setRetryCount] = useState(0);
  // Monotonic session counter — lets log lines across startRecording /
  // stopRecording / finalizeRecording be correlated without a UUID.
  const sessionIdRef = useRef(0);
  const [devices, setDevices] = useState<AudioInputDevice[]>([]);
  const [selectedDeviceId, setSelectedDeviceId] = useState<string>('');
  const [deviceMenuOpen, setDeviceMenuOpen] = useState(false);
  const gearButtonRef = useRef<HTMLButtonElement | null>(null);
  const [menuAnchor, setMenuAnchor] = useState<{ top: number; left: number } | null>(null);
  const recorderRef = useRef<MediaRecorder | null>(null);
  const streamRef = useRef<MediaStream | null>(null);
  const chunksRef = useRef<Blob[]>([]);
  // Tracks unmount so async callbacks (recorder.onstop, finalizeRecording)
  // don't fire setState/onSubmit on a dead component — without this, the
  // user navigating away mid-recording can dispatch an unintended message.
  const disposedRef = useRef(false);
  // Guards against rapid re-taps during the `getUserMedia` permission prompt.
  // Without this, two awaited `getUserMedia` calls can resolve back-to-back
  // and leave one of the granted streams orphaned (mic indicator stuck on).
  const startInFlightRef = useRef(false);
  // Recording timeout — auto-stops after MAX_RECORDING_MS.
  const recordingTimerRef = useRef<number | null>(null);
  const countdownRef = useRef<number | null>(null);
  const [remainingSecs, setRemainingSecs] = useState<number | null>(null);

  // If the component unmounts mid-record, release the mic so the OS indicator
  // doesn't get stuck on.
  useEffect(() => {
    disposedRef.current = false;
    return () => {
      disposedRef.current = true;
      clearRecordingTimer();
      // Detach onstop first — `recorder.stop()` below is what would fire it,
      // and we don't want finalizeRecording running post-unmount.
      if (recorderRef.current) recorderRef.current.onstop = null;
      stopStream();
      try {
        recorderRef.current?.stop();
      } catch {
        // recorder may already be inactive
      }
      recorderRef.current = null;
    };
  }, []);

  // Enumerate audio input devices when the selector is shown, and refresh the
  // list after the user grants mic permission (labels are hidden until then).
  useEffect(() => {
    if (!showDeviceSelector) return;
    async function loadDevices() {
      if (!navigator.mediaDevices?.enumerateDevices) return;
      try {
        const all = await navigator.mediaDevices.enumerateDevices();
        const inputs = all
          .filter(d => d.kind === 'audioinput')
          .map((d, i) => ({ deviceId: d.deviceId, label: d.label || `Microphone ${i + 1}` }));
        setDevices(inputs);
        // Keep the selected device valid; fall back to default.
        setSelectedDeviceId(prev =>
          inputs.some(d => d.deviceId === prev) ? prev : (inputs[0]?.deviceId ?? '')
        );
        composerLog('enumerated %d audio inputs', inputs.length);
      } catch (err) {
        composerLog('enumerateDevices failed: %s', err);
      }
    }
    void loadDevices();
    const onDeviceChange = () => void loadDevices();
    navigator.mediaDevices?.addEventListener?.('devicechange', onDeviceChange);
    return () => navigator.mediaDevices?.removeEventListener?.('devicechange', onDeviceChange);
  }, [showDeviceSelector]);

  // Spacebar = tap-to-toggle (#1471). Scoped to whatever surface mounts
  // this composer — today only the Human agent page. The listener lives
  // on the window so the user doesn't have to click the mascot stage
  // first, but it bails out when focus is inside an editable control or
  // a button so the shortcut never steals a keystroke from real input.
  useEffect(() => {
    function shouldIgnoreFocus(target: EventTarget | null): boolean {
      // Non-HTMLElement targets (SVG nodes, `document` itself) are
      // never text inputs, so the spacebar shortcut is safe to fire —
      // returning `false` here means "do not suppress".
      if (!(target instanceof HTMLElement)) return false;
      const tag = target.tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || tag === 'BUTTON') {
        return true;
      }
      // contenteditable surfaces (rich-text composers, ProseMirror,
      // etc.). `target.isContentEditable` is the right check in real
      // browsers because it walks the inheritance chain, but jsdom
      // doesn't compute the flag for plain `<div contenteditable>`,
      // so we additionally walk up via `closest` to cover both the
      // jsdom + production case.
      if (target.isContentEditable) return true;
      const editableAncestor = target.closest('[contenteditable]');
      if (editableAncestor instanceof HTMLElement) {
        const value = editableAncestor.getAttribute('contenteditable');
        // `contenteditable=""` and `contenteditable="true"` both mean
        // editable; `"false"` explicitly opts out.
        if (value === '' || value === 'true' || value === 'plaintext-only') {
          return true;
        }
      }
      return false;
    }

    function onKeyDown(event: KeyboardEvent) {
      // `event.code` is layout-independent — `'Space'` is the physical
      // bar key on every layout, where `event.key === ' '` would also
      // match remaps that shouldn't trigger voice. Stick to `code`.
      if (event.code !== 'Space') return;
      // Don't fight repeat-key autorepeat — the toggle should be edge-
      // triggered, not continuous.
      if (event.repeat) return;
      // Bare spacebar only. Modifier combinations (Shift-Space etc.) are
      // owned by the rest of the app and must keep flowing through.
      if (event.shiftKey || event.ctrlKey || event.metaKey || event.altKey) return;
      if (shouldIgnoreFocus(event.target ?? document.activeElement)) {
        composerLog(
          'spacebar ignored — focus inside editable target=%s',
          (event.target as HTMLElement | null)?.tagName ?? '<non-html>'
        );
        return;
      }
      // Prevent the default page-scroll behaviour and any focused-button
      // click activation (the user might be tabbed onto the mic button
      // itself, which would otherwise fire twice).
      event.preventDefault();
      if (disabled || state === 'transcribing') {
        composerLog('spacebar ignored — disabled=%s state=%s', disabled, state);
        return;
      }
      composerLog('spacebar toggle state=%s', state);
      if (state === 'recording') {
        stopRecording();
      } else {
        void startRecording();
      }
    }

    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
    // `state` is the only changing dependency the handler reads; the
    // refs are stable and `disabled` is captured via closure. Re-binding
    // on every state transition is cheap and keeps the snapshot in sync.
    // `startRecording` / `stopRecording` are plain function declarations
    // hoisted inside the component body — their identity is stable within
    // each render, so omitting them from the dep list is intentional, not
    // a stale-closure risk.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state, disabled]);

  function clearRecordingTimer() {
    if (recordingTimerRef.current != null) {
      window.clearTimeout(recordingTimerRef.current);
      recordingTimerRef.current = null;
    }
    if (countdownRef.current != null) {
      window.clearInterval(countdownRef.current);
      countdownRef.current = null;
    }
    setRemainingSecs(null);
  }

  function startRecordingTimer() {
    clearRecordingTimer();
    const totalSecs = Math.ceil(MAX_RECORDING_MS / 1000);
    let tick = totalSecs;
    setRemainingSecs(tick);
    countdownRef.current = window.setInterval(() => {
      tick = Math.max(0, tick - 1);
      setRemainingSecs(tick);
    }, 1000);
    recordingTimerRef.current = window.setTimeout(() => {
      composerLog('recording auto-stopped after %dms', MAX_RECORDING_MS);
      stopRecording();
    }, MAX_RECORDING_MS);
  }

  function stopStream() {
    if (streamRef.current) {
      for (const track of streamRef.current.getTracks()) {
        try {
          track.stop();
        } catch {
          // already stopped
        }
      }
      streamRef.current = null;
    }
  }

  async function startRecording() {
    if (state !== 'idle' || disabled || startInFlightRef.current) return;
    if (typeof navigator === 'undefined' || !navigator.mediaDevices?.getUserMedia) {
      onError?.(t('mic.unavailable'));
      return;
    }
    startInFlightRef.current = true;

    let stream: MediaStream;
    try {
      // Audio constraints tuned for STT accuracy:
      //   - mono: Scribe processes a single channel, stereo just doubles upload
      //   - 48kHz: matches Opus's native rate, no resample artifacts
      //   - {echo,noise,gain}: huge accuracy win on real-world mic input
      //     (untreated room noise + low-volume speech is the #1 reason
      //     transcription drops words in our flow)
      stream = await navigator.mediaDevices.getUserMedia({
        audio: {
          ...(selectedDeviceId ? { deviceId: { exact: selectedDeviceId } } : {}),
          channelCount: 1,
          sampleRate: 48000,
          echoCancellation: true,
          noiseSuppression: true,
          autoGainControl: true,
        },
      });
      // After the first successful grant, refresh device labels (they are
      // blank until the user has given permission).
      if (showDeviceSelector) {
        const all = await navigator.mediaDevices.enumerateDevices();
        const inputs = all
          .filter(d => d.kind === 'audioinput')
          .map((d, i) => ({ deviceId: d.deviceId, label: d.label || `Microphone ${i + 1}` }));
        setDevices(inputs);
      }
    } catch (err) {
      startInFlightRef.current = false;
      const msg = err instanceof Error ? err.message : String(err);
      composerLog('getUserMedia rejected: %s', msg);
      if (err instanceof DOMException) {
        if (err.name === 'NotAllowedError' || err.name === 'SecurityError') {
          onError?.(`${t('mic.permissionDenied')}: ${msg}`);
        } else if (err.name === 'NotFoundError' || err.name === 'OverconstrainedError') {
          onError?.(t('mic.deviceUnavailable'));
        } else if (err.name === 'NotReadableError') {
          onError?.(t('mic.deviceInUse'));
        } else {
          onError?.(`${t('mic.error')}: ${msg}`);
        }
      } else {
        onError?.(`${t('mic.error')}: ${msg}`);
      }
      return;
    }

    // Component unmounted while waiting for permission — release the granted
    // stream instead of leaking it (mic indicator would otherwise stay on).
    if (disposedRef.current) {
      startInFlightRef.current = false;
      stream.getTracks().forEach(t => t.stop());
      return;
    }

    const mime = pickRecorderMime();
    // 128kbps Opus is well above the threshold where Scribe's accuracy
    // plateaus; MediaRecorder's default for voice can be as low as 32kbps,
    // which audibly muddies consonants.
    const recorderOptions: MediaRecorderOptions = { audioBitsPerSecond: 128_000 };
    if (mime) recorderOptions.mimeType = mime;
    let recorder: MediaRecorder;
    try {
      recorder = new MediaRecorder(stream, recorderOptions);
    } catch (err) {
      stream.getTracks().forEach(t => t.stop());
      startInFlightRef.current = false;
      const msg = err instanceof Error ? err.message : String(err);
      onError?.(`${t('mic.failedToStartRecorder')}: ${msg}`);
      return;
    }

    chunksRef.current = [];
    recorder.ondataavailable = (e: BlobEvent) => {
      if (e.data && e.data.size > 0) chunksRef.current.push(e.data);
    };
    recorder.onstop = () => {
      void finalizeRecording();
    };

    streamRef.current = stream;
    recorderRef.current = recorder;
    recorder.start();
    setState('recording');
    setRetryCount(0);
    startInFlightRef.current = false;
    startRecordingTimer();
    const sessionId = ++sessionIdRef.current;
    composerLog(
      '[session:%d] recording started mime=%s device=%s',
      sessionId,
      recorder.mimeType || '(default)',
      selectedDeviceId || 'default'
    );
  }

  function stopRecording() {
    const recorder = recorderRef.current;
    if (!recorder || recorder.state === 'inactive') return;
    clearRecordingTimer();
    setState('transcribing');
    composerLog('[session:%d] recording stopped — transcribing', sessionIdRef.current);
    try {
      recorder.stop();
    } catch (err) {
      // If `stop()` throws, `onstop` never fires → finalizeRecording never
      // resets `state`, leaving the UI stuck on "Transcribing…". Recover here.
      composerLog('recorder.stop threw: %s', err);
      const msg = err instanceof Error ? err.message : String(err);
      onError?.(t('mic.failedToStopRecording').replace('{message}', msg));
      stopStream();
      recorderRef.current = null;
      setState('idle');
    }
  }

  async function finalizeRecording() {
    // Component was torn down mid-recording — clean up resources without
    // touching React state (which would log a warning) or `onSubmit`
    // (which would dispatch a message to a thread the user has left).
    if (disposedRef.current) {
      stopStream();
      recorderRef.current = null;
      chunksRef.current = [];
      return;
    }
    const recorder = recorderRef.current;
    recorderRef.current = null;
    stopStream();
    const chunks = chunksRef.current;
    chunksRef.current = [];

    const mime = recorder?.mimeType || 'audio/webm';
    const blob = new Blob(chunks, { type: mime });
    composerLog('recording stopped chunks=%d bytes=%d', chunks.length, blob.size);

    if (blob.size === 0) {
      setState('idle');
      onError?.(t('mic.noAudioCaptured'));
      return;
    }

    try {
      const transcript = await transcribeWithFallback(blob);
      if (disposedRef.current) return;
      if (!transcript) {
        onError?.(t('mic.noSpeechDetected'));
        setState('idle');
        return;
      }
      if (isLowConfidenceTranscript(transcript)) {
        composerLog('low-confidence transcript detected length=%d', transcript.length);
        onError?.(t('mic.lowConfidenceResult'));
        setState('idle');
        return;
      }
      await onSubmit(transcript);
    } catch (err) {
      if (disposedRef.current || isTranscriptionCancelledError(err)) {
        composerLog('transcribe cancelled after composer disposal');
        return;
      }
      const msg = err instanceof Error ? err.message : String(err);
      composerLog('transcribe failed: %s', msg);
      onError?.(t('mic.transcriptionFailed').replace('{message}', msg));
    } finally {
      if (!disposedRef.current) setState('idle');
    }
  }

  /**
   * Retry a transcription call up to `STT_MAX_RETRIES` times with
   * exponential backoff for transient errors. Permanent errors (stale
   * sidecar, empty blob) are rethrown immediately.
   *
   * `onRetry` is called before each retry attempt with the 1-based attempt
   * number so the UI can reflect progress (e.g. "Retrying… 1 of 2").
   */
  async function transcribeWithRetry(
    blob: Blob,
    label: string,
    onRetry?: (attempt: number) => void
  ): Promise<string> {
    const opts = language ? { language } : undefined;
    let lastErr: unknown;
    const throwIfDisposed = () => {
      if (disposedRef.current) throw new TranscriptionCancelledError();
    };
    for (let attempt = 0; attempt <= STT_MAX_RETRIES; attempt++) {
      throwIfDisposed();
      try {
        composerLog(
          '[session:%d] transcribe attempt=%s/%d bytes=%d mime=%s lang=%s',
          sessionIdRef.current,
          label,
          attempt,
          blob.size,
          blob.type,
          language || 'auto'
        );
        const transcript = await transcribeWithFactory(blob, opts);
        throwIfDisposed();
        composerLog(
          '[session:%d] transcribe ok path=%s attempt=%d',
          sessionIdRef.current,
          label,
          attempt
        );
        return transcript;
      } catch (err) {
        if (isTranscriptionCancelledError(err)) throw err;
        lastErr = err;
        const msg = err instanceof Error ? err.message : String(err);
        if (!isTransientError(err)) {
          composerLog(
            '[session:%d] transcribe permanent failure path=%s attempt=%d: %s',
            sessionIdRef.current,
            label,
            attempt,
            msg
          );
          throw err;
        }
        if (attempt < STT_MAX_RETRIES) {
          const delay = STT_RETRY_BASE_MS * Math.pow(2, attempt);
          composerLog(
            '[session:%d] transcribe transient failure path=%s attempt=%d retrying in %dms: %s',
            sessionIdRef.current,
            label,
            attempt,
            delay,
            msg
          );
          onRetry?.(attempt + 1);
          await new Promise(r => setTimeout(r, delay));
          throwIfDisposed();
        } else {
          composerLog(
            '[session:%d] transcribe exhausted retries path=%s attempt=%d: %s',
            sessionIdRef.current,
            label,
            attempt,
            msg
          );
        }
      }
    }
    throw lastErr;
  }

  /**
   * Send the recorder's native blob first (Opus-in-WebM ~3KB/sec) — Scribe
   * accepts it natively and it uploads ~30x faster than the 16kHz mono WAV
   * we used to transcode (~32KB/sec). If that ever fails (older STT
   * provider behind a feature flag, codec mismatch, …), re-encode to WAV
   * and retry so we don't regress correctness for the speed win. Each path
   * retries transient errors with exponential backoff.
   */
  async function transcribeWithFallback(blob: Blob): Promise<string> {
    const startedAt = Date.now();
    // Track the cumulative retry number across both codec paths so the label
    // always shows "Retrying… N of M" relative to the overall attempt count.
    let cumulativeRetries = 0;
    const handleRetry = (attempt: number) => {
      cumulativeRetries = attempt;
      setRetryCount(cumulativeRetries);
      composerLog('[session:%d] stt retry overall=%d', sessionIdRef.current, cumulativeRetries);
    };
    try {
      const text = await transcribeWithRetry(blob, 'native', handleRetry);
      composerLog(
        '[session:%d] transcribe ok path=native ms=%d',
        sessionIdRef.current,
        Math.round(Date.now() - startedAt)
      );
      return text;
    } catch (err) {
      if (isTranscriptionCancelledError(err)) throw err;
      const msg = err instanceof Error ? err.message : String(err);
      // Permanent errors don't benefit from a WAV re-encode — bail early.
      if (!isTransientError(err)) throw err;
      composerLog(
        '[session:%d] transcribe native path exhausted — falling back to wav: %s',
        sessionIdRef.current,
        msg
      );
      const reEncodeStart = Date.now();
      const wav = await encodeBlobToWav(blob);
      composerLog(
        '[session:%d] wav fallback bytes=%d encode_ms=%d',
        sessionIdRef.current,
        wav.size,
        Math.round(Date.now() - reEncodeStart)
      );
      // Snapshot before the WAV path so the WAV callback doesn't compound
      // against a value that handleRetry already mutated on native retries.
      const nativeRetries = cumulativeRetries;
      const text = await transcribeWithRetry(wav, 'wav', attempt => {
        handleRetry(nativeRetries + attempt);
      });
      composerLog(
        '[session:%d] transcribe ok path=wav-fallback total_ms=%d',
        sessionIdRef.current,
        Math.round(Date.now() - startedAt)
      );
      return text;
    }
  }

  const isRecording = state === 'recording';
  const isBusy = state === 'transcribing';
  const buttonDisabled = disabled || isBusy;

  const label = isBusy
    ? retryCount > 0
      ? t('mic.retryingTranscription')
          .replace('{attempt}', String(retryCount))
          .replace('{max}', String(STT_MAX_RETRIES * 2))
      : t('mic.transcribing')
    : isRecording
      ? remainingSecs != null && remainingSecs <= 10
        ? t('mic.tapToSendCountdown').replace('{seconds}', String(remainingSecs))
        : t('mic.tapToSend')
      : disabled
        ? t('mic.waitingForAgent')
        : t('mic.tapAndSpeak');

  const showDeviceMenuFab = showDeviceSelector && devices.length > 1;

  return (
    <div className="flex flex-col items-center gap-2">
      <div className="relative flex items-center justify-center gap-3">
        <button
          type="button"
          aria-label={isRecording ? t('mic.stopRecording') : t('mic.startRecording')}
          onClick={() => (isRecording ? stopRecording() : void startRecording())}
          disabled={buttonDisabled}
          className={`relative w-14 h-14 flex items-center justify-center rounded-full text-white shadow-soft transition-colors disabled:opacity-40 disabled:cursor-not-allowed ${
            isRecording ? 'bg-coral-500 hover:bg-coral-400' : 'bg-primary-500 hover:bg-primary-600'
          }`}>
          {isRecording && (
            <span className="absolute inset-0 rounded-full bg-coral-500/40 animate-ping" />
          )}
          {isBusy ? (
            <svg className="w-5 h-5 animate-spin" fill="none" viewBox="0 0 24 24">
              <circle
                className="opacity-25"
                cx="12"
                cy="12"
                r="10"
                stroke="currentColor"
                strokeWidth="4"
              />
              <path
                className="opacity-75"
                fill="currentColor"
                d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
              />
            </svg>
          ) : (
            <svg
              className="relative w-6 h-6"
              fill="none"
              stroke="currentColor"
              strokeWidth={1.8}
              viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                d="M12 18.75a6 6 0 006-6v-1.5m-6 7.5a6 6 0 01-6-6v-1.5m6 7.5v3.75m-3.75 0h7.5M12 15.75a3 3 0 01-3-3V4.5a3 3 0 116 0v8.25a3 3 0 01-3 3z"
              />
            </svg>
          )}
        </button>
        {showDeviceMenuFab && (
          <div className="relative">
            <button
              ref={gearButtonRef}
              type="button"
              aria-label={t('mic.deviceSelector') || 'Microphone device'}
              aria-expanded={deviceMenuOpen}
              onClick={() => {
                const rect = gearButtonRef.current?.getBoundingClientRect();
                if (rect) {
                  setMenuAnchor({ top: rect.bottom + 8, left: rect.left + rect.width / 2 });
                }
                setDeviceMenuOpen(open => !open);
              }}
              disabled={state !== 'idle'}
              className="w-8 h-8 flex items-center justify-center rounded-full border border-line bg-surface text-content-muted hover:text-content-secondary hover:border-line-strong dark:hover:border-neutral-600 transition-colors shadow-soft disabled:opacity-40 disabled:cursor-not-allowed">
              <svg
                className="w-4 h-4"
                fill="none"
                stroke="currentColor"
                strokeWidth={1.8}
                viewBox="0 0 24 24"
                aria-hidden>
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z"
                />
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"
                />
              </svg>
            </button>
            {deviceMenuOpen &&
              menuAnchor &&
              createPortal(
                <>
                  <div
                    className="fixed inset-0"
                    style={{ zIndex: 99998 }}
                    onClick={() => setDeviceMenuOpen(false)}
                    aria-hidden
                  />
                  <div
                    role="menu"
                    aria-label={t('mic.deviceSelector') || 'Microphone device'}
                    style={{
                      position: 'fixed',
                      top: menuAnchor.top,
                      left: menuAnchor.left,
                      transform: 'translateX(-50%)',
                      zIndex: 99999,
                    }}
                    className="w-64 rounded-xl border border-line bg-surface shadow-soft py-1">
                    {devices.map(d => {
                      const selected = d.deviceId === selectedDeviceId;
                      return (
                        <button
                          key={d.deviceId}
                          role="menuitemradio"
                          aria-checked={selected}
                          type="button"
                          onClick={() => {
                            setSelectedDeviceId(d.deviceId);
                            setDeviceMenuOpen(false);
                          }}
                          className={`w-full flex items-center gap-2 px-3 py-2 text-left text-xs transition-colors ${
                            selected
                              ? 'bg-primary-50 dark:bg-primary-900/20 text-content'
                              : 'text-content-secondary hover:bg-surface-hover'
                          }`}>
                          <span className="flex-1 min-w-0 truncate">{d.label}</span>
                          {selected && (
                            <svg
                              className="w-3.5 h-3.5 text-primary-500 flex-shrink-0"
                              fill="none"
                              stroke="currentColor"
                              strokeWidth={2.5}
                              viewBox="0 0 24 24"
                              aria-hidden>
                              <path
                                strokeLinecap="round"
                                strokeLinejoin="round"
                                d="M5 13l4 4L19 7"
                              />
                            </svg>
                          )}
                        </button>
                      );
                    })}
                  </div>
                </>,
                document.body
              )}
          </div>
        )}
        {onSwitchToText && (
          <button
            type="button"
            aria-label={t('chat.switchToText')}
            title={t('chat.switchToText')}
            onClick={onSwitchToText}
            disabled={state !== 'idle'}
            className="w-8 h-8 flex items-center justify-center rounded-full border border-line bg-surface text-content-muted hover:text-content-secondary hover:border-line-strong dark:hover:border-neutral-600 transition-colors shadow-soft disabled:opacity-40 disabled:cursor-not-allowed">
            <svg
              className="w-4 h-4"
              fill="none"
              stroke="currentColor"
              strokeWidth={1.8}
              viewBox="0 0 24 24"
              aria-hidden>
              <rect x="2" y="6" width="20" height="12" rx="2" ry="2" />
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                d="M6 10h.01M10 10h.01M14 10h.01M18 10h.01M7 14h10"
              />
            </svg>
          </button>
        )}
        <span className="text-xs text-content-muted select-none">{label}</span>
      </div>
    </div>
  );
}

export default MicComposer;
