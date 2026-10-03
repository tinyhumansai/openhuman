import { describe, expect, it } from 'vitest';

import { MEMORY_CHIPS, resolveMemoryChip } from './memoryChips';
import { fill, formatScore, formatTimestamp, metaFacts, parsePositiveInt } from './memoryFormat';

const t = (key: string) => key;

describe('resolveMemoryChip', () => {
  it('passes every v2 chip through', () => {
    for (const chip of MEMORY_CHIPS) expect(resolveMemoryChip(chip)).toBe(chip);
  });

  it('maps v1 sub-tabs', () => {
    expect(resolveMemoryChip('graph')).toBe('ask');
    expect(resolveMemoryChip('goals')).toBe('ask');
    expect(resolveMemoryChip('sources')).toBe('documents');
    expect(resolveMemoryChip('sync')).toBe('documents');
    expect(resolveMemoryChip('history')).toBe('documents');
  });

  it('returns null for nothing or an unknown value', () => {
    expect(resolveMemoryChip(null)).toBeNull();
    expect(resolveMemoryChip('')).toBeNull();
    expect(resolveMemoryChip('welcome')).toBeNull();
  });
});

describe('memoryFormat', () => {
  it('fills every placeholder occurrence', () => {
    expect(fill('{a} and {b} and {a}', { a: 1, b: 'x' })).toBe('1 and x and 1');
  });

  it('lists the useful meta facts in order, file winning over folder', () => {
    expect(
      metaFacts(
        { file_path: '/a/b.md', folder: '/a', repo: 'o/r', thread_id: 't1', url: 'https://u' },
        t
      ).map(f => f.key)
    ).toEqual(['file', 'repo', 'thread', 'url']);
    expect(metaFacts({ folder: '/a' }, t).map(f => f.key)).toEqual(['folder']);
    expect(metaFacts(undefined, t)).toEqual([]);
  });

  it('formats scores and timestamps defensively', () => {
    expect(formatScore(0.456)).toBe('0.46');
    expect(formatScore(null)).toBeNull();
    expect(formatScore(Number.NaN)).toBeNull();
    expect(formatTimestamp(null)).toBeNull();
    expect(formatTimestamp('not a date')).toBeNull();
    expect(formatTimestamp('2026-10-01T10:00:00Z')).toEqual(expect.any(String));
  });

  it('parses positive integers only', () => {
    expect(parsePositiveInt('12')).toBe(12);
    expect(parsePositiveInt(' 3 ')).toBe(3);
    expect(parsePositiveInt('0')).toBeNull();
    expect(parsePositiveInt('-1')).toBeNull();
    expect(parsePositiveInt('1.5')).toBeNull();
    expect(parsePositiveInt('')).toBeNull();
  });
});
