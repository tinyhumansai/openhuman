import { describe, expect, it } from 'vitest';

import { errorMessage } from './errorMessage';

const FALLBACK = 'Something went wrong';

describe('errorMessage', () => {
  it('reads the message off a real Error', () => {
    expect(errorMessage(new Error('boom'), FALLBACK)).toBe('boom');
  });

  // The case the helper exists for (#5900). `dispatch(thunk).unwrap()` rethrows
  // Redux Toolkit's SerializedError — a plain object, NOT an Error — so an
  // `instanceof` guard falls through to `String(value)`, which is
  // "[object Object]".
  it('reads the message off a SerializedError-shaped object', () => {
    const serialized = { name: 'Error', message: 'backend refused the profile', stack: '…' };

    expect(errorMessage(serialized, FALLBACK)).toBe('backend refused the profile');
    expect(errorMessage(serialized, FALLBACK)).not.toBe('[object Object]');
  });

  it('accepts a bare string, which is what rejectWithValue often carries', () => {
    expect(errorMessage('plain failure', FALLBACK)).toBe('plain failure');
  });

  it('trims surrounding whitespace', () => {
    expect(errorMessage(new Error('  padded  '), FALLBACK)).toBe('padded');
    expect(errorMessage('  bare  ', FALLBACK)).toBe('bare');
    expect(errorMessage({ message: '  object  ' }, FALLBACK)).toBe('object');
  });

  // An empty message is no more useful than none, and rendering it would put a
  // blank alert on screen — worse than the fallback, because it reads as "no
  // error". Same reasoning as `formatThreadCreateError` (threadSlice.ts:117-121).
  it.each([
    ['an Error with an empty message', new Error('')],
    ['an Error with a whitespace message', new Error('   ')],
    ['an object with an empty message', { message: '' }],
    ['an object with a whitespace message', { message: '   ' }],
    ['an empty string', ''],
    ['a whitespace string', '   '],
  ])('falls back for %s', (_label, value) => {
    expect(errorMessage(value, FALLBACK)).toBe(FALLBACK);
  });

  it.each([
    ['null', null],
    ['undefined', undefined],
    ['a number', 42],
    ['an object with no message', { code: 500 }],
    ['an object whose message is not a string', { message: { nested: true } }],
    ['an array', ['a', 'b']],
  ])('falls back for %s', (_label, value) => {
    expect(errorMessage(value, FALLBACK)).toBe(FALLBACK);
  });

  it('never returns the literal [object Object]', () => {
    for (const value of [{}, { message: {} }, { code: 1 }, Object.create(null)]) {
      expect(errorMessage(value, FALLBACK)).toBe(FALLBACK);
    }
  });
});
