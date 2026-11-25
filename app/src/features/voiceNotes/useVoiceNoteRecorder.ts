/**
 * useVoiceNoteRecorder - MediaRecorder capture for the voice-note composer.
 *
 * Unlike push-to-talk (`features/voice/pttAudio`), which hands the buffer
 * straight to STT, this keeps the recording as a draft the user can preview
 * and then send or discard. A coarse amplitude waveform is sampled while
 * recording so the bubble can draw bars without decoding the audio again.
 */
import { useCallback, useEffect, useRef, useState } from 'react';

export type RecorderPhase = 'idle' | 'recording' | 'recorded';

export interface VoiceNoteDraft {
  blob: Blob;
  durationMs: number;
  waveform: number[];
}

export const WAVEFORM_BARS = 40;
/** Longest recording; the recorder stops itself here. */
export const MAX_RECORDING_MS = 5 * 60 * 1000;
const SAMPLE_INTERVAL_MS = 100;

const MIME_PREFERENCE = [
  'audio/webm;codecs=opus',
  'audio/webm',
  'audio/ogg;codecs=opus',
  'audio/mp4',
];

function pickMimeType(): string | undefined {
  if (typeof MediaRecorder === 'undefined') return undefined;
  return MIME_PREFERENCE.find(m => MediaRecorder.isTypeSupported(m));
}

/** Resample raw amplitude samples to `bars` values in 0..1. */
export function reduceWaveform(samples: number[], bars: number = WAVEFORM_BARS): number[] {
  if (samples.length === 0) return [];
  const out: number[] = [];
  for (let i = 0; i < bars; i++) {
    const from = Math.floor((i * samples.length) / bars);
    const to = Math.max(from + 1, Math.floor(((i + 1) * samples.length) / bars));
    let peak = 0;
    for (let j = from; j < Math.min(to, samples.length); j++) peak = Math.max(peak, samples[j]);
    out.push(Math.min(1, Math.round(peak * 100) / 100));
  }
  return out;
}

export interface VoiceNoteRecorder {
  phase: RecorderPhase;
  elapsedMs: number;
  draft: VoiceNoteDraft | null;
  /** Rejects when the microphone is unavailable or permission is denied. */
  start: () => Promise<void>;
  stop: () => void;
  /** Drop the recording or the draft. */
  discard: () => void;
}

export function useVoiceNoteRecorder(): VoiceNoteRecorder {
  const [phase, setPhase] = useState<RecorderPhase>('idle');
  const [elapsedMs, setElapsedMs] = useState(0);
  const [draft, setDraft] = useState<VoiceNoteDraft | null>(null);
  const live = useRef<{
    recorder: MediaRecorder;
    stream: MediaStream;
    chunks: Blob[];
    startedAt: number;
    samples: number[];
    timer: ReturnType<typeof setInterval>;
    audioCtx?: AudioContext;
    analyser?: AnalyserNode;
    cancelled: boolean;
  } | null>(null);

  const teardown = useCallback(() => {
    const session = live.current;
    if (!session) return;
    live.current = null;
    clearInterval(session.timer);
    for (const track of session.stream.getTracks()) {
      try {
        track.stop();
      } catch {
        /* ignore */
      }
    }
    void session.audioCtx?.close().catch(() => undefined);
  }, []);

  const start = useCallback(async () => {
    if (live.current) return;
    const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
    const mimeType = pickMimeType();
    const recorder = new MediaRecorder(stream, mimeType ? { mimeType } : undefined);
    const chunks: Blob[] = [];
    const samples: number[] = [];
    let audioCtx: AudioContext | undefined;
    let analyser: AnalyserNode | undefined;
    try {
      if (typeof AudioContext !== 'undefined') {
        audioCtx = new AudioContext();
        analyser = audioCtx.createAnalyser();
        analyser.fftSize = 512;
        audioCtx.createMediaStreamSource(stream).connect(analyser);
      }
    } catch {
      analyser = undefined;
    }
    const buffer = new Uint8Array(analyser?.fftSize ?? 0);
    const startedAt = window.performance.now();

    const timer = setInterval(() => {
      const session = live.current;
      if (!session) return;
      const elapsed = window.performance.now() - session.startedAt;
      setElapsedMs(elapsed);
      if (session.analyser) {
        session.analyser.getByteTimeDomainData(buffer);
        let peak = 0;
        for (const v of buffer) peak = Math.max(peak, Math.abs(v - 128) / 128);
        session.samples.push(peak);
      }
      if (elapsed >= MAX_RECORDING_MS && session.recorder.state !== 'inactive') {
        session.recorder.stop();
      }
    }, SAMPLE_INTERVAL_MS);

    recorder.addEventListener('dataavailable', (e: BlobEvent) => {
      if (e.data && e.data.size > 0) chunks.push(e.data);
    });
    recorder.addEventListener('stop', () => {
      const session = live.current;
      const cancelled = session?.cancelled ?? true;
      const durationMs = Math.round(window.performance.now() - startedAt);
      const recorded = session ? [...session.samples] : [];
      teardown();
      if (cancelled || chunks.length === 0) {
        setPhase('idle');
        setElapsedMs(0);
        return;
      }
      const blob = new Blob(chunks, { type: recorder.mimeType || mimeType || 'audio/webm' });
      setDraft({ blob, durationMs, waveform: reduceWaveform(recorded) });
      setElapsedMs(durationMs);
      setPhase('recorded');
    });

    live.current = {
      recorder,
      stream,
      chunks,
      startedAt,
      samples,
      timer,
      audioCtx,
      analyser,
      cancelled: false,
    };
    setDraft(null);
    setElapsedMs(0);
    setPhase('recording');
    recorder.start();
  }, [teardown]);

  const stop = useCallback(() => {
    const session = live.current;
    if (session && session.recorder.state !== 'inactive') session.recorder.stop();
  }, []);

  const discard = useCallback(() => {
    const session = live.current;
    if (session) {
      session.cancelled = true;
      if (session.recorder.state !== 'inactive') session.recorder.stop();
      else teardown();
    }
    setDraft(null);
    setElapsedMs(0);
    setPhase('idle');
  }, [teardown]);

  // Release the microphone if the composer unmounts mid-recording.
  useEffect(
    () => () => {
      const session = live.current;
      if (session) {
        session.cancelled = true;
        if (session.recorder.state !== 'inactive') session.recorder.stop();
        teardown();
      }
    },
    [teardown]
  );

  return { phase, elapsedMs, draft, start, stop, discard };
}
