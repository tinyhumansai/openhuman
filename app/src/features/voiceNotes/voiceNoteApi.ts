/**
 * voiceNoteApi - core RPC wrappers for in-app voice notes.
 *
 * The core owns the audio: a recording is stored through
 * `channel.web_voice_note_store` and a thread row only keeps `{ noteId, mime }`
 * in its `extraMetadata.voiceNote`. Assistant spoken replies use the same
 * store, so one reader serves both directions.
 */
import type { ContentPart } from '../../services/chatWire';
import { callCoreRpc } from '../../services/coreRpcClient';
import { openhumanVoiceTranscribeBytes } from '../../utils/tauriCommands/voice';

/** `extraMetadata.voiceNote` as persisted on a thread row. */
export interface VoiceNoteMeta {
  noteId: string;
  mime?: string;
  durationMs?: number;
  /** Normalised 0..1 bar heights, user recordings only. */
  waveform?: number[];
  /** Provider timeline saved with synthesized replies; absent on legacy notes. */
  visemes?: Array<{ viseme: string; start_ms: number; end_ms: number }>;
}

/** `source` the composer stamps on a turn that began as a voice note. */
export const VOICE_NOTE_SOURCE = 'voice_note';

/** Unwrap the core's Outcome `{ logs, result }` wire envelope for note RPCs. */
function unwrapVoiceNoteResult<T>(value: T | { result?: T }): T {
  if (value && typeof value === 'object' && 'result' in value) {
    const result = (value as { result?: T }).result;
    if (result && typeof result === 'object') return result;
  }
  return value as T;
}

export function extensionForMime(mime: string): string {
  if (mime.includes('ogg')) return 'ogg';
  if (mime.includes('mp4')) return 'm4a';
  return 'webm';
}

export function bytesToBase64(bytes: Uint8Array): string {
  let binary = '';
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}

/** Keep the recorded media on the typed web-chat wire beside its transcript. */
export async function voiceNoteContentPart(
  noteId: string,
  mime: string,
  blob: Blob,
  durationMs: number,
  waveform: number[]
): Promise<ContentPart> {
  const contentType = mime || blob.type || 'audio/webm';
  return {
    kind: 'voice_note',
    voiceNote: {
      codec: null,
      durationMs,
      waveform,
      pushToTalk: false,
      transcript: null,
      media: {
        opaqueId: noteId,
        kind: 'audio',
        contentType,
        durationMs,
        sizeBytes: blob.size,
        codec: null,
        fileName: null,
        height: null,
        localCachePath: null,
        path: null,
        platformMessageId: null,
        transcribed: false,
        url: null,
        width: null,
      },
    },
  };
}

export function base64ToBytes(b64: string): Uint8Array {
  const binary = atob(b64);
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i);
  return out;
}

/** Narrow untyped `extraMetadata` to a voice-note reference, or `null`. */
export function readVoiceNoteMeta(extraMetadata: unknown): VoiceNoteMeta | null {
  if (!extraMetadata || typeof extraMetadata !== 'object') return null;
  const raw = (extraMetadata as Record<string, unknown>).voiceNote;
  if (!raw || typeof raw !== 'object') return null;
  const note = raw as Record<string, unknown>;
  if (typeof note.noteId !== 'string' || note.noteId.length === 0) return null;
  return {
    noteId: note.noteId,
    mime: typeof note.mime === 'string' ? note.mime : undefined,
    durationMs: typeof note.durationMs === 'number' ? note.durationMs : undefined,
    waveform: Array.isArray(note.waveform)
      ? note.waveform.filter((v): v is number => typeof v === 'number')
      : undefined,
    visemes: Array.isArray(note.visemes)
      ? note.visemes.filter(
          (frame): frame is { viseme: string; start_ms: number; end_ms: number } =>
            !!frame &&
            typeof frame === 'object' &&
            typeof (frame as Record<string, unknown>).viseme === 'string' &&
            Number.isFinite((frame as Record<string, unknown>).start_ms) &&
            Number.isFinite((frame as Record<string, unknown>).end_ms)
        )
      : undefined,
  };
}

export async function storeVoiceNote(noteId: string, blob: Blob): Promise<{ mime: string }> {
  const bytes = new Uint8Array(await blob.arrayBuffer());
  const mime = blob.type || 'audio/webm';
  const response = await callCoreRpc<{ mime?: string } | { result?: { mime?: string } }>({
    method: 'openhuman.channel_web_voice_note_store',
    params: { note_id: noteId, audio_base64: bytesToBase64(bytes), mime },
  });
  const result = unwrapVoiceNoteResult(response);
  return { mime: result?.mime ?? mime };
}

/** Fetch a stored note's audio; `null` when the core has none. */
export async function readVoiceNote(noteId: string): Promise<Blob | null> {
  const response = await callCoreRpc<
    | { audio_base64?: string | null; mime?: string | null }
    | { result?: { audio_base64?: string | null; mime?: string | null } }
  >({ method: 'openhuman.channel_web_voice_note_read', params: { note_id: noteId } });
  const result = unwrapVoiceNoteResult(response);
  if (!result?.audio_base64) return null;
  const bytes = base64ToBytes(result.audio_base64);
  return new Blob([bytes as BlobPart], { type: result.mime || 'audio/webm' });
}

/** Run the configured STT engine over a recording. Empty string when nothing was heard. */
export async function transcribeVoiceNote(blob: Blob): Promise<string> {
  const bytes = Array.from(new Uint8Array(await blob.arrayBuffer()));
  const result = await openhumanVoiceTranscribeBytes(
    bytes,
    extensionForMime(blob.type || 'audio/webm'),
    undefined,
    false
  );
  return (result?.text ?? result?.raw_text ?? '').trim();
}

export function formatDuration(ms: number | undefined): string {
  const total = Math.max(0, Math.round((ms ?? 0) / 1000));
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m}:${String(s).padStart(2, '0')}`;
}
