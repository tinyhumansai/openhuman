import { describe, expect, it } from 'vitest';

import { VISEMES } from '../Mascot/visemes';
import { framesProduceMotion, selectVisemeSource, splitIntoSentences } from './ttsQueue';

describe('splitIntoSentences', () => {
  it('carves out complete sentences and keeps the trailing fragment as rest', () => {
    const { sentences, rest } = splitIntoSentences('Hello world. How are you? ');
    expect(sentences).toEqual(['Hello world.', 'How are you?']);
    expect(rest.trim()).toBe('');
  });

  it('leaves a sentence with no trailing whitespace buffered as rest', () => {
    // The final sentence only completes once whitespace (more text) or an
    // end-of-turn flush follows it — mid-stream it stays in rest.
    const { sentences, rest } = splitIntoSentences('Hello world. How are you?');
    expect(sentences).toEqual(['Hello world.']);
    expect(rest.trim()).toBe('How are you?');
  });

  it('does not split on decimals or version numbers', () => {
    const { sentences, rest } = splitIntoSentences('Pi is 3.14 exactly. ');
    expect(sentences).toEqual(['Pi is 3.14 exactly.']);
    expect(rest.trim()).toBe('');
  });

  it('returns no sentences when the buffer has no completed sentence', () => {
    const { sentences, rest } = splitIntoSentences('an unfinished thought');
    expect(sentences).toEqual([]);
    expect(rest).toBe('an unfinished thought');
  });

  it('handles runs of terminators and multiple sentences', () => {
    const { sentences, rest } = splitIntoSentences('Wait!! Really?? Yes. ');
    expect(sentences).toEqual(['Wait!!', 'Really??', 'Yes.']);
    expect(rest.trim()).toBe('');
  });
});

describe('framesProduceMotion', () => {
  it('is true when at least one frame maps to a non-REST shape', () => {
    expect(framesProduceMotion([{ viseme: 'aa', start_ms: 0, end_ms: 100 }])).toBe(true);
  });

  it('is false when every frame maps to REST', () => {
    expect(
      framesProduceMotion([
        { viseme: '???', start_ms: 0, end_ms: 100 },
        { viseme: 'unknown', start_ms: 100, end_ms: 200 },
      ])
    ).toBe(false);
  });
});

describe('selectVisemeSource', () => {
  it('keeps backend visemes when they animate and carry a usable timeline', () => {
    const visemes = [
      { viseme: 'aa', start_ms: 0, end_ms: 200 },
      { viseme: 'PP', start_ms: 200, end_ms: 400 },
    ];
    const result = selectVisemeSource({ visemes });
    expect(result.source).toBe('visemes');
    expect(result.frames).toEqual(visemes);
  });

  it('falls through to alignment when the viseme codes are all unknown', () => {
    const result = selectVisemeSource({
      visemes: [
        { viseme: '???', start_ms: 0, end_ms: 100 },
        { viseme: 'unknown', start_ms: 100, end_ms: 200 },
      ],
      alignment: [{ char: 'a', start_ms: 0, end_ms: 50 }],
    });
    expect(result.source).toBe('alignment');
    expect(result.frames.length).toBeGreaterThan(0);
  });

  it('falls through to alignment when starts are degenerate (all zero)', () => {
    const result = selectVisemeSource({
      visemes: [
        { viseme: 'aa', start_ms: 0, end_ms: 80 },
        { viseme: 'PP', start_ms: 0, end_ms: 80 },
      ],
      alignment: [{ char: 'h', start_ms: 0, end_ms: 40 }],
    });
    expect(result.source).toBe('alignment');
  });

  it('returns an empty list (procedural signal) when nothing usable is present', () => {
    const result = selectVisemeSource({ visemes: [] });
    expect(result.source).toBe('visemes');
    expect(result.frames).toEqual([]);
  });

  it('never maps an unknown code to a real shape', () => {
    // Guards the regression the all-REST detector exists for.
    const result = selectVisemeSource({ visemes: [{ viseme: 'zzz', start_ms: 0, end_ms: 100 }] });
    expect(result.frames).toEqual([]);
    // sanity: a real code would have produced motion
    expect(framesProduceMotion([{ viseme: 'aa', start_ms: 0, end_ms: 100 }])).toBe(true);
    expect(VISEMES.REST).toBeDefined();
  });
});
