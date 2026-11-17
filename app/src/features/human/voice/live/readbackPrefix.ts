/**
 * Instruction prefix for "speak-back". A slow voice turn (e.g. an email summary)
 * is acknowledged aloud, finishes in the background, and its result is delivered
 * to chat AND pushed to the renderer as a `voice_speak` event. While a live voice
 * session is open, the renderer sends it back into the session as a `text` frame
 * wrapped with this prefix so the agent reads it verbatim. MUST match
 * `VOICE_READBACK_PREFIX` in `voice/realtime_harness/prompt.rs`, which uses it to
 * avoid re-arming speak-back on the read-back turn (loop guard) — pinned by
 * `readbackPrefix.contract.test.ts`.
 */
export const READBACK_PREFIX =
  'Please read the following to me, word for word, and say nothing else:';
