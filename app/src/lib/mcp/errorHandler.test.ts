/**
 * Unit tests for MCP error handling utilities
 */
import { describe, expect, it, vi } from 'vitest';

import { ErrorCategory, logAndFormatError, withErrorHandling } from './errorHandler';
import { ValidationError } from './validation';

describe('logAndFormatError', () => {
  it('returns an MCPToolResult with isError=true', () => {
    const result = logAndFormatError('myFn', new Error('boom'));
    expect(result.isError).toBe(true);
    expect(result.content).toHaveLength(1);
    expect(result.content[0].type).toBe('text');
  });

  it('includes the error code in the user message for generic errors', () => {
    const result = logAndFormatError('myFn', new Error('internal'));
    expect(result.content[0].text).toMatch(/code:/);
  });

  it('exposes the ValidationError message directly to the user', () => {
    const err = new ValidationError('chat_id must be a positive integer');
    const result = logAndFormatError('myFn', err, ErrorCategory.VALIDATION);
    expect(result.content[0].text).toBe('chat_id must be a positive integer');
  });

  it('includes a VALIDATION-001 error code when category is VALIDATION', () => {
    const result = logAndFormatError('sendMsg', new Error('oops'), ErrorCategory.VALIDATION);
    // ValidationError path is not triggered — but category affects the code
    expect(result.content[0].text).toMatch(/VALIDATION-001/);
  });

  it('uses the supplied category as a prefix for non-validation errors', () => {
    const result = logAndFormatError('sendMsg', new Error('fail'), ErrorCategory.CHAT);
    expect(result.content[0].text).toMatch(/CHAT/);
  });

  it('produces a stable code for the same function name', () => {
    const r1 = logAndFormatError('stableFn', new Error('a'));
    const r2 = logAndFormatError('stableFn', new Error('b'));
    // Strip the leading text, just compare the code portion
    const code1 = r1.content[0].text.match(/code: ([^)]+)/)?.[1];
    const code2 = r2.content[0].text.match(/code: ([^)]+)/)?.[1];
    expect(code1).toBe(code2);
  });
});

describe('withErrorHandling', () => {
  it('returns the wrapped function result when no error is thrown', async () => {
    const fn = vi
      .fn()
      .mockResolvedValue({ content: [{ type: 'text' as const, text: 'ok' }], isError: false });
    const wrapped = withErrorHandling(fn);
    const result = await wrapped();
    expect(result.isError).toBe(false);
    expect(result.content[0].text).toBe('ok');
  });

  it('catches a thrown Error and returns an error MCPToolResult', async () => {
    const fn = vi.fn().mockRejectedValue(new Error('network down'));
    const wrapped = withErrorHandling(fn, ErrorCategory.MSG);
    const result = await wrapped();
    expect(result.isError).toBe(true);
    expect(result.content[0].text).toMatch(/code:/);
  });

  it('catches a thrown non-Error value and wraps it', async () => {
    const fn = vi.fn().mockRejectedValue('string error');
    const wrapped = withErrorHandling(fn);
    const result = await wrapped();
    expect(result.isError).toBe(true);
  });

  it('exposes ValidationError message through the wrapper', async () => {
    const fn = vi.fn().mockRejectedValue(new ValidationError('param must be string'));
    const wrapped = withErrorHandling(fn, ErrorCategory.VALIDATION);
    const result = await wrapped();
    expect(result.isError).toBe(true);
    expect(result.content[0].text).toBe('param must be string');
  });
});
