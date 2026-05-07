import debug from 'debug';

import { callCoreRpc } from '../../../services/coreRpcClient';

const sttLog = debug('human:stt');

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
    // Issue #1289: an "unknown method" error means the bundled core
    // sidecar is older than the frontend (e.g. a stale dev build, or a
    // cached binary the desktop auto-update hasn't refreshed yet).
    // The raw "unknown method: openhuman.voice_cloud_transcribe" string
    // is opaque to end users — surface an actionable message instead.
    const msg = err instanceof Error ? err.message : String(err);
    if (msg.includes('unknown method')) {
      sttLog('transcribe rpc stale-sidecar path hit; rewriting unknown-method error: %s', msg);
      throw new Error(
        'Voice transcription is unavailable in this build. Restart the OpenHuman desktop app to pick up the latest core sidecar.'
      );
    }
    sttLog('transcribe rpc failed (passthrough): %O', err);
    throw err;
  }
  const text = result?.text?.trim() ?? '';
  sttLog('transcribed chars=%d rpc_ms=%d', text.length, Math.round(Date.now() - rpcStart));
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
