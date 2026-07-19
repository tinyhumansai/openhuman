import { describe, expect, it } from 'vitest';

import { parseToolFailure } from './chatRuntimeSlice';

describe('parseToolFailure', () => {
  it('parses a well-formed snake_case wire object into camelCase', () => {
    const parsed = parseToolFailure({
      class: 'MissingPermission',
      category: 'NeedsUserConfirmation',
      recoverable: false,
      cause_plain: "OpenHuman doesn't have permission to do this yet.",
      next_action: 'Grant the permission it needs, then try again.',
    });
    expect(parsed).toEqual({
      class: 'MissingPermission',
      category: 'NeedsUserConfirmation',
      recoverable: false,
      causePlain: "OpenHuman doesn't have permission to do this yet.",
      nextAction: 'Grant the permission it needs, then try again.',
    });
  });

  it('accepts camelCase keys (persisted round-trip)', () => {
    const parsed = parseToolFailure({
      class: 'Timeout',
      category: 'Recoverable',
      recoverable: true,
      causePlain: 'Took too long.',
      nextAction: 'Retry.',
    });
    expect(parsed?.causePlain).toBe('Took too long.');
    expect(parsed?.nextAction).toBe('Retry.');
    expect(parsed?.recoverable).toBe(true);
  });

  it('defaults recoverable to false when absent or non-boolean', () => {
    const parsed = parseToolFailure({
      class: 'Unknown',
      category: 'Recoverable',
      cause_plain: 'Oops.',
      next_action: 'Try again.',
    });
    expect(parsed?.recoverable).toBe(false);
  });

  it.each([
    ['null', null],
    ['undefined', undefined],
    ['a string', 'MissingPermission'],
    ['a number', 42],
    ['an empty object', {}],
    ['missing next_action', { class: 'X', category: 'Y', cause_plain: 'c' }],
    ['non-string class', { class: 1, category: 'Y', cause_plain: 'c', next_action: 'n' }],
  ])('returns undefined for garbage input (%s)', (_label, input) => {
    expect(parseToolFailure(input)).toBeUndefined();
  });
});
