import debug from 'debug';
import { useEffect, useRef, useState } from 'react';

import { transcribeCloud } from './voice/sttClient';
import { encodeBlobToWav } from './voice/wavEncoder';

const composerLog = debug('human:mic-composer');

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

export interface MicCloudComposerProps {
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
}

type RecordingState = 'idle' | 'recording' | 'transcribing';

/**
 * Tap-to-toggle mic composer for the mascot page. Captures audio via the
 * browser's `MediaRecorder`, hands the resulting Blob to the cloud STT proxy
 * (`openhuman.voice_cloud_transcribe`), then forwards the transcript through
 * `onSubmit` so it joins the agent's normal send pipeline.
 *
 * Single button, single decision: tap once to start recording, tap again to
 * stop and send. No textarea — that's the whole point of the mascot tab.
 */
export function MicCloudComposer({
  disabled,
  onSubmit,
  onError,
  language = 'en',
}: MicCloudComposerProps) {
  const [state, setState] = useState<RecordingState>('idle');
  const recorderRef = useRef<MediaRecorder | null>(null);
  const streamRef = useRef<MediaStream | null>(null);
  const chunksRef = useRef<Blob[]>([]);
  // Tracks unmount so async callbacks (recorder.onstop, finalizeRecording)
  // don't fire setState/onSubmit on a dead component — without this, the
  // user navigating away mid-recording can dispatch an unintended message.
  const disposedRef = useRef(false);

  // If the component unmounts mid-record, release the mic so the OS indicator
  // doesn't get stuck on.
  useEffect(() => {
    disposedRef.current = false;
    return () => {
      disposedRef.current = true;
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
    if (state !== 'idle' || disabled) return;
    if (typeof navigator === 'undefined' || !navigator.mediaDevices?.getUserMedia) {
      onError?.('Microphone access is not available in this runtime.');
      return;
    }

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
          channelCount: 1,
          sampleRate: 48000,
          echoCancellation: true,
          noiseSuppression: true,
          autoGainControl: true,
        },
      });
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      composerLog('getUserMedia rejected: %s', msg);
      onError?.(`Microphone permission denied: ${msg}`);
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
      const msg = err instanceof Error ? err.message : String(err);
      onError?.(`Failed to start recorder: ${msg}`);
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
    composerLog('recording started mime=%s', recorder.mimeType || '(default)');
  }

  function stopRecording() {
    const recorder = recorderRef.current;
    if (!recorder || recorder.state === 'inactive') return;
    setState('transcribing');
    try {
      recorder.stop();
    } catch (err) {
      // If `stop()` throws, `onstop` never fires → finalizeRecording never
      // resets `state`, leaving the UI stuck on "Transcribing…". Recover here.
      composerLog('recorder.stop threw: %s', err);
      const msg = err instanceof Error ? err.message : String(err);
      onError?.(`Failed to stop recording: ${msg}`);
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
      onError?.('No audio captured. Try holding the mic a little longer.');
      return;
    }

    try {
      const transcript = await transcribeWithFallback(blob);
      if (!transcript) {
        onError?.('No speech detected. Try again.');
        setState('idle');
        return;
      }
      await onSubmit(transcript);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      composerLog('transcribe failed: %s', msg);
      onError?.(`Voice transcription failed: ${msg}`);
    } finally {
      setState('idle');
    }
  }

  /**
   * Send the recorder's native blob first (Opus-in-WebM ~3KB/sec) — Scribe
   * accepts it natively and it uploads ~30x faster than the 16kHz mono WAV
   * we used to transcode (~32KB/sec). If that ever fails (older STT
   * provider behind a feature flag, codec mismatch, …), retry once with a
   * re-encoded WAV so we don't regress correctness for the speed win.
   */
  async function transcribeWithFallback(blob: Blob): Promise<string> {
    const startedAt = Date.now();
    const opts = language ? { language } : undefined;
    try {
      composerLog(
        'transcribe attempt=native bytes=%d mime=%s lang=%s',
        blob.size,
        blob.type,
        language || 'auto'
      );
      const text = await transcribeCloud(blob, opts);
      composerLog('transcribe ok attempt=native ms=%d', Math.round(Date.now() - startedAt));
      return text;
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      composerLog('transcribe failed attempt=native — falling back to wav: %s', msg);
      const reEncodeStart = Date.now();
      const wav = await encodeBlobToWav(blob);
      composerLog(
        'wav fallback bytes=%d encode_ms=%d',
        wav.size,
        Math.round(Date.now() - reEncodeStart)
      );
      const text = await transcribeCloud(wav, opts);
      composerLog(
        'transcribe ok attempt=wav-fallback total_ms=%d',
        Math.round(Date.now() - startedAt)
      );
      return text;
    }
  }

  const isRecording = state === 'recording';
  const isBusy = state === 'transcribing';
  const buttonDisabled = disabled || isBusy;

  const label = isBusy
    ? 'Transcribing…'
    : isRecording
      ? 'Tap to send'
      : disabled
        ? 'Waiting for the agent…'
        : 'Tap and speak';

  return (
    <div className="flex items-center justify-center gap-3">
      <button
        type="button"
        aria-label={isRecording ? 'Stop recording and send' : 'Start recording'}
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
      <span className="text-xs text-stone-500 select-none">{label}</span>
    </div>
  );
}

export default MicCloudComposer;
