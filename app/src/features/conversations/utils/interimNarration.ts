/**
 * Which interim narration bubbles a finished turn has superseded.
 *
 * The agent emits `extraMetadata.isInterim` messages while it works ("Let me
 * get the data for both.", "The HTML is hard to parse. Let me search for a
 * clean table.") — these are live progress, not content. Once the turn delivers
 * its real answer they are superseded; left unfiltered they pile up
 * permanently, wedging several stale "Let me…" bubbles between the question and
 * the answer on every multi-tool turn.
 *
 * The rule is per TURN, keyed on that turn having produced a final
 * (non-interim) agent message:
 *
 * - turn still in flight → no final message yet → narration stays visible
 * - turn answered        → narration hidden, the answer speaks for it
 * - turn died first      → narration kept; it is the only record of what ran
 *
 * Deriving this from the answer's existence rather than a turn-active flag is
 * what makes the third case work, and keeps the helper pure (no lifecycle
 * plumbing, trivially testable).
 *
 * Nothing is deleted — callers use this to filter the rendered list only. The
 * messages stay persisted and reachable via "View full agent process Source".
 */
import type { ThreadMessage } from '../../../types/thread';

/**
 * Indexes into `messages` whose interim narration is superseded and should not
 * render. Returns indexes (not ids) so callers can filter positionally without
 * assuming ids are present or unique.
 */
export function supersededInterimIndexes(messages: readonly ThreadMessage[]): Set<number> {
  const hidden = new Set<number>();
  let segmentStart = 0;

  // A turn spans (previous user message, next user message]. Closing a segment
  // hides its narration only when that segment also produced a real answer.
  const closeSegment = (end: number) => {
    let hasFinalAnswer = false;
    for (let i = segmentStart; i < end; i += 1) {
      const msg = messages[i];
      if (msg.sender === 'agent' && !msg.extraMetadata?.isInterim) {
        hasFinalAnswer = true;
        break;
      }
    }
    if (!hasFinalAnswer) return;
    for (let i = segmentStart; i < end; i += 1) {
      if (messages[i].extraMetadata?.isInterim) hidden.add(i);
    }
  };

  messages.forEach((msg, index) => {
    if (msg.sender === 'user') {
      closeSegment(index);
      segmentStart = index + 1;
    }
  });
  closeSegment(messages.length);

  return hidden;
}
