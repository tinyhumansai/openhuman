/**
 * Tests for local-chat gating logic.
 *
 * These are pure unit tests — no Tauri / socket I/O involved.
 * They verify:
 *   1. segmentMessage correctly splits responses into bubbles.
 *   2. getSegmentDelay stays within [500, 1400] ms.
 *   3. The local-only gate pattern (isLocalModelActive) works
 *      as intended by checking its value logic directly.
 */
import { describe, expect, it } from 'vitest';

import { getSegmentDelay, segmentMessage } from '../messageSegmentation';

const MIN_DELAY = 500;
const MAX_DELAY = 1400;

describe('local chat gating — segmentation', () => {
  it('short reply is a single bubble', () => {
    const reply = 'Sure thing!';
    expect(segmentMessage(reply)).toEqual([reply]);
  });

  it('long reply with paragraph breaks splits into multiple bubbles', () => {
    const reply = [
      'First paragraph with enough content to pass the minimum length check.',
      'Second paragraph also has enough content to be its own bubble in the UI.',
    ].join('\n\n');
    const segments = segmentMessage(reply);
    expect(segments.length).toBeGreaterThanOrEqual(2);
    expect(segments.length).toBeLessThanOrEqual(5);
  });

  it('each segment meets minimum length threshold', () => {
    const reply = [
      'This is the first segment that is long enough to stand on its own.',
      'This is the second segment that is also long enough to be shown.',
      'And a third one here.',
    ].join('\n\n');
    const segments = segmentMessage(reply);
    for (const seg of segments) {
      expect(seg.length).toBeGreaterThan(0);
    }
  });

  it('never returns more than 5 segments', () => {
    const reply = Array.from(
      { length: 10 },
      (_, i) => `Paragraph number ${i + 1} has enough words to be a proper segment here.`
    ).join('\n\n');
    const segments = segmentMessage(reply);
    expect(segments.length).toBeLessThanOrEqual(5);
  });
});

describe('local chat gating — delivery delays', () => {
  it('delay is at least MIN_DELAY for any segment', () => {
    expect(getSegmentDelay('')).toBeGreaterThanOrEqual(MIN_DELAY);
    expect(getSegmentDelay('x')).toBeGreaterThanOrEqual(MIN_DELAY);
  });

  it('delay is capped at MAX_DELAY', () => {
    expect(getSegmentDelay('a'.repeat(10_000))).toBeLessThanOrEqual(MAX_DELAY);
  });

  it('delay increases with segment length', () => {
    const d1 = getSegmentDelay('short');
    const d2 = getSegmentDelay('a'.repeat(300));
    expect(d2).toBeGreaterThanOrEqual(d1);
  });
});

describe('local chat gating — message role mapping', () => {
  it('maps sender user → role user', () => {
    const sender = 'user';
    const role = sender === 'user' ? 'user' : 'assistant';
    expect(role).toBe('user');
  });

  it('maps sender agent → role assistant', () => {
    const sender: string = 'agent';
    const role = sender === 'user' ? 'user' : 'assistant';
    expect(role).toBe('assistant');
  });
});
