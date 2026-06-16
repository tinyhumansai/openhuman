/**
 * pttTranscribe — speech-to-text adapter for pttService.
 *
 * Reuses the existing `openhuman.voice_transcribe_bytes` RPC (see
 * `src/openhuman/voice/ops.rs`). The Rust side handles cloud + whisper.cpp
 * routing based on the user's `stt_provider` setting and applies optional
 * LLM cleanup, so the renderer only needs to push raw bytes.
 *
 * The `extension` hint comes from `pttAudio.lastRecordedExtension()` —
 * MediaRecorder negotiates webm/opus on every modern desktop browser.
 */
import { openhumanVoiceTranscribeBytes } from '../../utils/tauriCommands/voice';
import { lastRecordedExtension } from './pttAudio';

/**
 * Encode the buffer as a byte array for JSON-RPC transport. The wire format
 * expects `Vec<u8>` deserialized from a number array; serde-json doesn't
 * support binary natively over JSON-RPC.
 *
 * This is O(N) memory and CPU. For a 10s @ ~16 kbps opus blob (~20 KB) it's
 * cheap; if PTT recordings grow past ~5 MB we should swap to base64 or a
 * dedicated upload endpoint.
 */
function bufferToByteArray(buf: ArrayBuffer): number[] {
  const view = new Uint8Array(buf);
  const out = new Array<number>(view.byteLength);
  for (let i = 0; i < view.byteLength; i++) {
    out[i] = view[i];
  }
  return out;
}

export async function transcribePttAudio(buf: ArrayBuffer): Promise<string> {
  if (buf.byteLength === 0) return '';
  const extension = lastRecordedExtension();
  const bytes = bufferToByteArray(buf);
  const result = await openhumanVoiceTranscribeBytes(
    bytes,
    extension,
    /* context */ undefined,
    /* skipCleanup */ false
  );
  // `result.text` is the cleaned-up version (LLM-polished when enabled);
  // `raw_text` is the unfiltered whisper output. Prefer text but fall back.
  const text = (result?.text ?? result?.raw_text ?? '').trim();
  return text;
}
