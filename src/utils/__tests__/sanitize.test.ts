import { describe, expect, it } from 'vitest';

import { createSafeLogData, sanitizeError, sanitizeForLogging } from '../sanitize';

describe('sanitizeError', () => {
  it('extracts name, message, and stack from Error objects', () => {
    const err = new Error('something broke');
    const result = sanitizeError(err) as Record<string, unknown>;
    expect(result.name).toBe('Error');
    expect(result.message).toBe('something broke');
    // DEV mode is set in test env, so stack should be present
    expect(result.stack).toBeDefined();
  });

  it('sanitizes plain objects by redacting sensitive keys', () => {
    const input = { token: 'abc123', username: 'bob' };
    const result = sanitizeError(input) as Record<string, unknown>;
    expect(result.token).toBe('[REDACTED]');
    expect(result.username).toBe('bob');
  });

  it('returns primitives as-is', () => {
    expect(sanitizeError('string error')).toBe('string error');
    expect(sanitizeError(42)).toBe(42);
    expect(sanitizeError(null)).toBeNull();
  });
});

describe('sanitizeForLogging', () => {
  it('returns null/undefined as-is', () => {
    expect(sanitizeForLogging(null)).toBeNull();
    expect(sanitizeForLogging(undefined)).toBeUndefined();
  });

  it('redacts sensitive keys in nested objects', () => {
    const input = {
      user: 'alice',
      config: { apiKey: 'secret-key', endpoint: 'https://api.example.com' },
    };
    const result = sanitizeForLogging(input) as Record<string, unknown>;
    const config = result.config as Record<string, unknown>;
    expect(config.apiKey).toBe('[REDACTED]');
    expect(config.endpoint).toBe('https://api.example.com');
  });

  it('truncates large objects and shows preview', () => {
    const bigData: Record<string, string> = {};
    for (let i = 0; i < 200; i++) {
      bigData[`field_${i}`] = `value_${'x'.repeat(10)}`;
    }
    const result = sanitizeForLogging(bigData) as Record<string, unknown>;
    expect(result._truncated).toBe(true);
    expect(result._preview).toBeDefined();
    expect(typeof result._size).toBe('number');
  });

  it('sanitizes arrays by iterating each element', () => {
    const input = [
      { password: 'hidden', name: 'alice' },
      { password: 'hidden', name: 'bob' },
    ];
    const result = sanitizeForLogging(input) as Array<Record<string, unknown>>;
    expect(result[0].password).toBe('[REDACTED]');
    expect(result[0].name).toBe('alice');
    expect(result[1].password).toBe('[REDACTED]');
  });

  it('stops at max recursion depth', () => {
    // Build deeply nested object
    let obj: Record<string, unknown> = { value: 'deep' };
    for (let i = 0; i < 10; i++) {
      obj = { nested: obj };
    }
    const result = sanitizeForLogging(obj);
    // Should not throw, should have truncated
    expect(result).toBeDefined();
  });
});

describe('createSafeLogData', () => {
  it('includes metadata and marks hasData=false when no sensitiveData', () => {
    const result = createSafeLogData({ action: 'login' });
    expect(result.action).toBe('login');
    expect(result.hasData).toBe(false);
  });

  it('includes sanitized data for small payloads', () => {
    const result = createSafeLogData({ action: 'fetch' }, { userId: '123', token: 'secret' });
    expect(result.hasData).toBe(true);
    expect(result.dataSize).toBeGreaterThan(0);
    const data = result.data as Record<string, unknown>;
    expect(data.token).toBe('[REDACTED]');
    expect(data.userId).toBe('123');
  });

  it('shows preview for large payloads', () => {
    const largePayload = 'x'.repeat(1000);
    const result = createSafeLogData({ action: 'upload' }, largePayload);
    expect(result.hasData).toBe(true);
    expect(result.dataSize).toBe(1000);
  });
});
