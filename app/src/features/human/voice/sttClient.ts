import debug from 'debug';

import { callCoreRpc } from '../../../services/coreRpcClient';

const sttLog = debug('human:stt');

/**
 * Stable identifier for "the core has no voice domain compiled in" (#4901).
 *
 * Callers must branch on this code rather than on the message text: the message
 * is untranslated developer/log copy, and the UI is responsible for rendering
 * translated copy via `useT()` (see `MicComposer`'s `mic.voiceNotCompiled`).
 */
export const VOICE_NOT_COMPILED_CODE = 'voice_not_compiled';

/**
 * Thrown when the core answers `unknown method` for a `voice_*` RPC — i.e. the
 * core was compiled without the `voice` feature, so the domain is absent from
 * the binary entirely (#4901).
 *
 * The `message` is English on purpose: it is what reaches debug logs and Sentry.
 * User-facing copy is resolved from `code` at the UI boundary.
 *
 * Keep the `unavailable in this build` substring: `MicComposer`'s
 * `PERMANENT_ERROR_PATTERNS` matches on it to skip the retry/backoff loop,
 * which can never succeed for a compile-time gate.
 */
export class VoiceNotCompiledError extends Error {
  readonly code = VOICE_NOT_COMPILED_CODE;

  constructor() {
    super(
      'Voice transcription is unavailable in this build — the voice module was not compiled into the app. ' +
        'Update OpenHuman to the latest version; restarting will not help.'
    );
    this.name = 'VoiceNotCompiledError';
  }
}

/**
 * Duck-typed on `code` rather than `instanceof`: the error crosses async +
 * retry boundaries, and structured-clone/serialization paths can strip the
 * prototype while preserving own properties.
 */
export function isVoiceNotCompiledError(err: unknown): err is VoiceNotCompiledError {
  return (
    typeof err === 'object' &&
    err !== null &&
    (err as { code?: unknown }).code === VOICE_NOT_COMPILED_CODE
  );
}

export interface CloudTranscribeOptions {
  /** Override the backend STT model id. Default is whatever the backend
   *  resolves `whisper-v1` to today. */
  model?: string;
  /** BCP-47 language hint, e.g. `'en'`. */
  language?: string;
  /** Defaults derived from the recorded blob. */
  mimeType?: string;
  fileName?: string;
}

export interface CloudTranscribeResult {
  text: string;
}

/**
 * Transcribe a recorded audio blob via the Rust core's cloud STT proxy.
 *
 * The blob is read into a base64 string and shipped over JSON-RPC; the core
 * decodes it and POSTs `multipart/form-data` to the hosted backend's
 * `/openai/v1/audio/transcriptions` endpoint. Going through the core keeps
 * the provider API key off the desktop app and reuses the same auth flow as
 * `synthesizeSpeech`.
 */
export async function transcribeCloud(
  blob: Blob,
  opts: CloudTranscribeOptions = {}
): Promise<string> {
  if (!blob || blob.size === 0) {
    throw new Error('audio blob is empty');
  }
  const encodeStart = Date.now();
  const audio_base64 = await blobToBase64(blob);
  const encodeMs = Math.round(Date.now() - encodeStart);

  const params: Record<string, unknown> = { audio_base64 };
  // MediaRecorder mime types include codec parameters (e.g. `audio/webm;codecs=opus`)
  // — the backend's allow-list expects the bare type, so strip the suffix.
  const mime = (opts.mimeType ?? blob.type ?? 'audio/webm').split(';')[0].trim() || 'audio/webm';
  params.mime_type = mime;
  params.file_name = opts.fileName ?? `audio.${guessExtension(mime)}`;
  if (opts.model) params.model = opts.model;
  if (opts.language) params.language = opts.language;

  sttLog(
    'transcribe bytes=%d mime=%s base64_ms=%d (b64_size=%d)',
    blob.size,
    mime,
    encodeMs,
    audio_base64.length
  );

  const rpcStart = Date.now();
  let result: CloudTranscribeResult;
  try {
    result = await callCoreRpc<CloudTranscribeResult>({
      method: 'openhuman.voice_cloud_transcribe',
      params,
    });
  } catch (err) {
    // An "unknown method" error means the core serving this app was built
    // without the `voice` Cargo feature, so the `openhuman.voice_*`
    // controllers were never registered (#4901). This is a compile-time
    // property of the binary — restarting cannot change it, which is why the
    // old #1289-era "restart to pick up the latest core sidecar" copy was
    // unactionable (and the sidecar itself is gone since #1061).
    const msg = err instanceof Error ? err.message : String(err);
    if (msg.includes('unknown method')) {
      sttLog('[voice-stt] transcribe rpc: voice domain absent from core build: %s', msg);
      throw new VoiceNotCompiledError();
    }
    sttLog('transcribe rpc failed (passthrough): %O', err);
    throw err;
  }
  const text = result?.text?.trim() ?? '';
  sttLog('transcribed chars=%d rpc_ms=%d', text.length, Math.round(Date.now() - rpcStart));
  return text;
}

export interface FactoryTranscribeOptions {
  /** BCP-47 language hint, e.g. `'en'`. */
  language?: string;
  /** Override the server-side provider resolution (`'cloud'` | `'whisper'`).
   *  When unset the core reads `config.local_ai.stt_provider`. */
  provider?: 'cloud' | 'whisper';
  /** Whisper model id (whisper branch only). */
  model?: string;
  /** Defaults derived from the recorded blob. */
  mimeType?: string;
  fileName?: string;
}

export interface FactoryTranscribeResult {
  text: string;
  /** Provider that actually ran ('cloud' or 'whisper'). */
  provider: string;
}

/**
 * Factory-dispatched transcription. Hits `openhuman.voice_stt_dispatch`
 * — the core resolves the provider from config (or `opts.provider` when
 * the caller forces one). Returns the transcript only; the renderer
 * surfaces the provider id via debug logs.
 *
 * Goes through the same base64 encoding path as `transcribeCloud` so the
 * MicComposer can swap implementations without re-tooling the recorder.
 */
export async function transcribeWithFactory(
  blob: Blob,
  opts: FactoryTranscribeOptions = {}
): Promise<string> {
  if (!blob || blob.size === 0) {
    throw new Error('audio blob is empty');
  }
  const encodeStart = Date.now();
  const audio_base64 = await blobToBase64(blob);
  const encodeMs = Math.round(Date.now() - encodeStart);

  const params: Record<string, unknown> = { audio_base64 };
  const mime = (opts.mimeType ?? blob.type ?? 'audio/webm').split(';')[0].trim() || 'audio/webm';
  params.mime_type = mime;
  params.file_name = opts.fileName ?? `audio.${guessExtension(mime)}`;
  if (opts.provider) params.provider = opts.provider;
  if (opts.model) params.model = opts.model;
  if (opts.language) params.language = opts.language;

  sttLog(
    '[voice-stt] transcribe-factory bytes=%d mime=%s provider=%s base64_ms=%d',
    blob.size,
    mime,
    opts.provider ?? '<config>',
    encodeMs
  );

  const rpcStart = Date.now();
  let result: FactoryTranscribeResult;
  try {
    result = await callCoreRpc<FactoryTranscribeResult>({
      method: 'openhuman.voice_stt_dispatch',
      params,
    });
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    if (msg.includes('unknown method')) {
      sttLog('[voice-stt] dispatch: voice domain absent from core build: %s', msg);
      throw new VoiceNotCompiledError();
    }
    sttLog('[voice-stt] dispatch failed (passthrough): %O', err);
    throw err;
  }
  const text = result?.text?.trim() ?? '';
  sttLog(
    '[voice-stt] transcribed provider=%s chars=%d rpc_ms=%d',
    result?.provider ?? '<unknown>',
    text.length,
    Math.round(Date.now() - rpcStart)
  );
  return text;
}

async function blobToBase64(blob: Blob): Promise<string> {
  const buf = await blob.arrayBuffer();
  const bytes = new Uint8Array(buf);
  // Chunked to avoid `Maximum call stack` on large clips when spread into
  // String.fromCharCode in one go.
  const CHUNK = 0x8000;
  let binary = '';
  for (let i = 0; i < bytes.length; i += CHUNK) {
    binary += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
  }
  return btoa(binary);
}

function guessExtension(mime: string): string {
  switch (mime) {
    case 'audio/webm':
    case 'video/webm':
      return 'webm';
    case 'audio/ogg':
      return 'ogg';
    case 'audio/mpeg':
      return 'mp3';
    case 'audio/wav':
    case 'audio/x-wav':
      return 'wav';
    case 'audio/mp4':
    case 'audio/x-m4a':
      return 'm4a';
    case 'audio/flac':
      return 'flac';
    default:
      return 'webm';
  }
}
