/**
 * pttAudio — push-to-talk mic-capture adapter for pttService.
 *
 * Dictation's existing recorder lives in the Rust core (rdev-driven, fed by
 * the audio_capture domain) and surfaces results asynchronously over a
 * dedicated socket — it is not exposed as a reusable JS function that returns
 * a buffer. Rather than refactor that flow, we use a self-contained
 * MediaRecorder in the renderer. The captured audio is sent straight to the
 * existing `voice_transcribe_bytes` RPC (see `pttTranscribe.ts`), so we still
 * reuse the core's STT path; only the capture layer is renderer-owned.
 *
 * Module-level state is intentional — the singleton matches `pttService`'s
 * lifecycle (one active PTT session at a time, owned by `PttHotkeyManager`).
 * `cancel` is idempotent so the watchdog / preempt paths can call it freely.
 */
import type { FinalizedAudio } from '../../services/pttService';

interface Recorder {
  recorder: MediaRecorder;
  stream: MediaStream;
  chunks: Blob[];
  startedAt: number;
}

let active: Recorder | null = null;
let lastMimeType: string | undefined;

function pickMimeType(): string | undefined {
  // Prefer webm/opus — small, broadly supported. whisper.cpp + cloud STT both
  // accept it via ffmpeg decode; the core's `extension` hint is "webm".
  const preferred = ['audio/webm;codecs=opus', 'audio/webm', 'audio/ogg;codecs=opus', 'audio/mp4'];
  for (const mime of preferred) {
    if (typeof MediaRecorder !== 'undefined' && MediaRecorder.isTypeSupported(mime)) {
      return mime;
    }
  }
  return undefined;
}

function stopTracks(stream: MediaStream): void {
  for (const track of stream.getTracks()) {
    try {
      track.stop();
    } catch {
      /* ignore */
    }
  }
}

export async function startPttAudio(opts: { sessionTag: string }): Promise<void> {
  // If a prior session was abandoned without a finalize/cancel, free it now
  // so we don't leak the mic.
  if (active) {
    console.debug('[ptt-audio] startPttAudio called with active recorder — cancelling first');
    await cancelPttAudio();
  }

  const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
  const mimeType = pickMimeType();
  const recorder = new MediaRecorder(stream, mimeType ? { mimeType } : undefined);
  const chunks: Blob[] = [];

  recorder.addEventListener('dataavailable', (e: BlobEvent) => {
    if (e.data && e.data.size > 0) chunks.push(e.data);
  });

  active = { recorder, stream, chunks, startedAt: window.performance.now() };
  lastMimeType = mimeType ?? recorder.mimeType ?? undefined;
  recorder.start();
  console.debug('[ptt-audio] started', { sessionTag: opts.sessionTag, mimeType });
}

export async function finalizePttAudio(): Promise<FinalizedAudio> {
  if (!active) {
    throw new Error('[ptt-audio] finalize called with no active recorder');
  }
  const session = active;
  active = null;

  const done = new Promise<void>(resolve => {
    if (session.recorder.state === 'inactive') {
      resolve();
      return;
    }
    session.recorder.addEventListener('stop', () => resolve(), { once: true });
  });
  try {
    if (session.recorder.state !== 'inactive') session.recorder.stop();
  } catch (err) {
    console.warn('[ptt-audio] recorder.stop() threw', err);
  }
  await done;
  stopTracks(session.stream);

  const blob = new Blob(session.chunks, { type: session.recorder.mimeType || 'audio/webm' });
  const buffer = await blob.arrayBuffer();
  const durationMs = Math.round(window.performance.now() - session.startedAt);
  console.debug('[ptt-audio] finalized', { durationMs, bytes: buffer.byteLength });
  return { buffer, durationMs };
}

export async function cancelPttAudio(): Promise<void> {
  if (!active) return;
  const session = active;
  active = null;
  try {
    if (session.recorder.state !== 'inactive') session.recorder.stop();
  } catch {
    /* ignore */
  }
  stopTracks(session.stream);
  console.debug('[ptt-audio] cancelled');
}

/**
 * Maps the last-used MIME type to an extension string the core's
 * `voice_transcribe_bytes` RPC accepts. Persists across `finalizePttAudio`
 * (which clears `active`) so the transcribe step still gets the right hint.
 */
export function lastRecordedExtension(): string {
  const mime = lastMimeType ?? '';
  if (mime.includes('webm')) return 'webm';
  if (mime.includes('ogg')) return 'ogg';
  if (mime.includes('mp4')) return 'm4a';
  return 'webm';
}
