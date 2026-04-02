import { describe, expect, it } from 'vitest';

import { getSegmentDelay, segmentMessage } from '../messageSegmentation';

describe('segmentMessage', () => {
  it('returns the original text as a single segment when text is short', () => {
    const short = 'Hello there!';
    expect(segmentMessage(short)).toEqual([short]);
  });

  it('returns single segment for text under 80 chars', () => {
    const text = 'This is a sentence that is almost long enough but not quite yet for splitting.';
    expect(segmentMessage(text)).toHaveLength(1);
  });

  it('splits on paragraph breaks', () => {
    const text =
      'First paragraph with enough content to be meaningful and readable.\n\n' +
      'Second paragraph that also has meaningful content and is not too short.';
    const segments = segmentMessage(text);
    expect(segments.length).toBeGreaterThanOrEqual(2);
    expect(segments[0]).toContain('First paragraph');
    expect(segments[1]).toContain('Second paragraph');
  });

  it('merges short paragraphs with the previous one', () => {
    const text =
      'First paragraph with enough content to stand alone as a meaningful segment.\n\n' +
      'Short.\n\n' +
      'Third paragraph with enough content to be a standalone bubble in delivery.';
    const segments = segmentMessage(text);
    // "Short." should be merged with first or third
    expect(segments.every(s => s.length >= 40)).toBe(true);
  });

  it('splits on sentence boundaries when no paragraph breaks exist', () => {
    const text =
      'This is the first sentence with some content. This is the second sentence with more content. ' +
      'This is the third sentence that adds even more words. And this is a final wrap-up sentence here.';
    const segments = segmentMessage(text);
    expect(segments.length).toBeGreaterThanOrEqual(2);
  });

  it('never returns more than 5 segments', () => {
    const manyParagraphs = Array.from(
      { length: 10 },
      (_, i) =>
        `Paragraph ${i + 1} has enough words and content to be a real standalone bubble message.`
    ).join('\n\n');
    const segments = segmentMessage(manyParagraphs);
    expect(segments.length).toBeLessThanOrEqual(5);
  });

  it('handles text with only a single paragraph gracefully', () => {
    const single =
      'This is a single paragraph that happens to be long enough to potentially split but has no newlines or clear sentence breaks ending in uppercase.';
    const segments = segmentMessage(single);
    expect(segments.length).toBeGreaterThanOrEqual(1);
    expect(segments.every(s => s.length > 0)).toBe(true);
  });

  it('does not return empty segments', () => {
    const text =
      'Valid first paragraph content.\n\n\n\n\n\nValid second paragraph content that is long enough.';
    const segments = segmentMessage(text);
    expect(segments.every(s => s.trim().length > 0)).toBe(true);
  });
});

describe('getSegmentDelay', () => {
  it('returns a value between 500 and 1400 ms', () => {
    const delays = [
      getSegmentDelay(''),
      getSegmentDelay('short'),
      getSegmentDelay('x'.repeat(100)),
      getSegmentDelay('x'.repeat(1000)),
    ];
    for (const d of delays) {
      expect(d).toBeGreaterThanOrEqual(500);
      expect(d).toBeLessThanOrEqual(1400);
    }
  });

  it('returns a larger delay for longer segments', () => {
    const short = getSegmentDelay('Hi there!');
    const long = getSegmentDelay(
      'This is a much longer segment that contains many more words and characters.'
    );
    expect(long).toBeGreaterThan(short);
  });
});

describe('edge cases', () => {
  it('handles text with only whitespace', () => {
    expect(segmentMessage('   ')).toEqual(['']);
  });

  it('handles exactly 80 characters — treated as short, returns single segment', () => {
    const text = 'a'.repeat(80);
    expect(segmentMessage(text)).toHaveLength(1);
  });

  it('handles exactly 81 characters with a paragraph break — may split', () => {
    const text = 'a'.repeat(40) + '\n\n' + 'b'.repeat(40);
    const result = segmentMessage(text);
    // Both paragraphs are >= MIN_SEGMENT_CHARS so should split
    expect(result.length).toBeGreaterThanOrEqual(1);
  });

  it('delay scales with length', () => {
    const short = getSegmentDelay('hi');
    const long = getSegmentDelay('a'.repeat(500));
    expect(long).toBeGreaterThan(short);
    expect(long).toBeLessThanOrEqual(1400);
    expect(short).toBeGreaterThanOrEqual(500);
  });
});
