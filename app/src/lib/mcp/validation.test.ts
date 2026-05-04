/**
 * Unit tests for MCP validation utilities
 */
import { describe, expect, it } from 'vitest';

import {
  validateId,
  validateIdList,
  validateOptionalId,
  validatePositiveInt,
  ValidationError,
} from './validation';

describe('ValidationError', () => {
  it('is an instance of Error with name ValidationError', () => {
    const err = new ValidationError('bad input');
    expect(err).toBeInstanceOf(Error);
    expect(err.name).toBe('ValidationError');
    expect(err.message).toBe('bad input');
  });
});

describe('validateId', () => {
  it('accepts a positive integer', () => {
    expect(validateId(123, 'chat_id')).toBe(123);
  });

  it('accepts a negative integer', () => {
    expect(validateId(-100, 'chat_id')).toBe(-100);
  });

  it('accepts a numeric string and returns the parsed integer', () => {
    expect(validateId('456', 'user_id')).toBe(456);
  });

  it('accepts a bare username string and prepends @', () => {
    expect(validateId('username', 'user_id')).toBe('@username');
  });

  it('accepts a username string that already starts with @', () => {
    expect(validateId('@username', 'user_id')).toBe('@username');
  });

  it('throws ValidationError for a non-integer number', () => {
    expect(() => validateId(3.14, 'chat_id')).toThrow(ValidationError);
  });

  it('throws ValidationError for an invalid string that is not a number or username', () => {
    expect(() => validateId('!!!', 'chat_id')).toThrow(ValidationError);
  });

  it('throws ValidationError for null', () => {
    expect(() => validateId(null, 'chat_id')).toThrow(ValidationError);
  });

  it('throws ValidationError for boolean', () => {
    expect(() => validateId(true, 'chat_id')).toThrow(ValidationError);
  });
});

describe('validateIdList', () => {
  it('validates an array of integers', () => {
    expect(validateIdList([1, 2, 3], 'ids')).toEqual([1, 2, 3]);
  });

  it('validates a mixed array of integers and usernames', () => {
    expect(validateIdList([42, 'alice'], 'ids')).toEqual([42, '@alice']);
  });

  it('throws ValidationError when value is not an array', () => {
    expect(() => validateIdList('not-an-array', 'ids')).toThrow(ValidationError);
  });

  it('throws ValidationError when an element is invalid', () => {
    expect(() => validateIdList([1, null], 'ids')).toThrow(ValidationError);
  });
});

describe('validatePositiveInt', () => {
  it('accepts a positive integer', () => {
    expect(validatePositiveInt(5, 'msg_id')).toBe(5);
  });

  it('accepts a positive integer string', () => {
    expect(validatePositiveInt('10', 'msg_id')).toBe(10);
  });

  it('throws ValidationError for zero', () => {
    expect(() => validatePositiveInt(0, 'msg_id')).toThrow(ValidationError);
  });

  it('throws ValidationError for a negative number', () => {
    expect(() => validatePositiveInt(-1, 'msg_id')).toThrow(ValidationError);
  });

  it('throws ValidationError for a float', () => {
    expect(() => validatePositiveInt(1.5, 'msg_id')).toThrow(ValidationError);
  });

  it('throws ValidationError for a non-numeric string', () => {
    expect(() => validatePositiveInt('abc', 'msg_id')).toThrow(ValidationError);
  });
});

describe('validateOptionalId', () => {
  it('returns undefined for undefined input', () => {
    expect(validateOptionalId(undefined, 'chat_id')).toBeUndefined();
  });

  it('returns undefined for null input', () => {
    expect(validateOptionalId(null, 'chat_id')).toBeUndefined();
  });

  it('delegates to validateId for a present value', () => {
    expect(validateOptionalId(99, 'chat_id')).toBe(99);
  });

  it('throws ValidationError when the present value is invalid', () => {
    expect(() => validateOptionalId(true, 'chat_id')).toThrow(ValidationError);
  });
});
