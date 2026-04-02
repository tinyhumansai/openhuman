/**
 * messageSegmentation — splits AI responses into natural chat bubbles.
 *
 * Gate: only called when local model is active. Cloud/API paths bypass this entirely.
 */

const MIN_SEGMENT_CHARS = 40;
const MAX_SEGMENTS = 5;

/**
 * Split `text` into an array of segments suitable for multi-bubble delivery.
 *
 * Priority order:
 *  1. Paragraph breaks (\n\n) — highest-fidelity natural split.
 *  2. Sentence-ending punctuation (. ! ?) followed by space + uppercase.
 *  3. Fall back to the whole text as a single segment.
 *
 * Always returns at least one element. Returns the original text as `[text]`
 * when it's too short or cannot be meaningfully split.
 */
export function segmentMessage(text: string): string[] {
  const trimmed = text.trim();

  // Don't bother segmenting short messages
  if (trimmed.length < 80) return [trimmed];

  // --- Strategy 1: paragraph splits ---
  const paragraphs = trimmed
    .split(/\n\n+/)
    .map(p => p.trim())
    .filter(p => p.length > 0);

  if (paragraphs.length >= 2) {
    const merged = mergeTooShort(paragraphs, '\n\n');
    if (merged.length >= 2) return merged.slice(0, MAX_SEGMENTS);
  }

  // --- Strategy 2: sentence splits ---
  const sentences = splitSentences(trimmed);
  if (sentences.length >= 2) {
    const grouped = groupSentences(sentences);
    if (grouped.length >= 2) return grouped.slice(0, MAX_SEGMENTS);
  }

  // --- Fallback: single bubble ---
  return [trimmed];
}

/**
 * Estimate a natural inter-bubble delay in milliseconds.
 * Scales loosely with the length of the segment just delivered.
 * Bounded: min 500ms, max 1 400ms.
 */
export function getSegmentDelay(segment: string): number {
  const base = 500;
  const perChar = 1.5;
  return Math.min(base + segment.length * perChar, 1400);
}

// ─── helpers ─────────────────────────────────────────────────────────────────

/** Merge adjacent items that are shorter than MIN_SEGMENT_CHARS. */
function mergeTooShort(parts: string[], joiner: string): string[] {
  const result: string[] = [];
  for (const part of parts) {
    if (result.length > 0 && part.length < MIN_SEGMENT_CHARS) {
      result[result.length - 1] += joiner + part;
    } else {
      result.push(part);
    }
  }
  return result;
}

/**
 * Split on sentence-ending punctuation (. ! ?) followed by a space and
 * an uppercase letter. Uses a manual loop to avoid lookbehind assertions
 * that may not be available in all WebKit versions.
 */
function splitSentences(text: string): string[] {
  const parts: string[] = [];
  let current = '';

  for (let i = 0; i < text.length; i++) {
    current += text[i];
    const ch = text[i];
    const next1 = text[i + 1];
    const next2 = text[i + 2];

    if (
      (ch === '.' || ch === '!' || ch === '?') &&
      next1 === ' ' &&
      next2 !== undefined &&
      next2 >= 'A' &&
      next2 <= 'Z'
    ) {
      parts.push(current.trim());
      current = '';
      i++; // skip the space
    }
  }

  if (current.trim().length > 0) parts.push(current.trim());
  return parts.filter(p => p.length > 0);
}

/**
 * Group individual sentences into at most MAX_SEGMENTS bubbles.
 * Tries to aim for 2–3 bubbles for readability.
 */
function groupSentences(sentences: string[]): string[] {
  const targetCount = Math.min(3, Math.ceil(sentences.length / 2));
  const groupSize = Math.ceil(sentences.length / targetCount);
  const groups: string[] = [];

  for (let i = 0; i < sentences.length; i += groupSize) {
    groups.push(sentences.slice(i, i + groupSize).join(' '));
  }

  return groups.filter(g => g.length >= MIN_SEGMENT_CHARS);
}
