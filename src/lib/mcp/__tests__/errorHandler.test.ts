/**
 * Tests for error handling utilities
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { ErrorCategory, logAndFormatError, withErrorHandling } from '../errorHandler';
import type { MCPToolResult } from '../types';
import { ValidationError } from '../validation';

describe('logAndFormatError', () => {
  let consoleErrorSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
  });

  afterEach(() => {
    consoleErrorSpy.mockRestore();
  });

  it('should pass through ValidationError message directly', () => {
    const error = new ValidationError('Invalid user ID format');
    const result = logAndFormatError('testFunction', error);

    expect(result).toEqual({
      content: [{ type: 'text', text: 'Invalid user ID format' }],
      isError: true,
    });
    expect(consoleErrorSpy).toHaveBeenCalled();
  });

  it('should generate error code for generic Error', () => {
    const error = new Error('Something went wrong');
    const result = logAndFormatError('testFunction', error);

    expect(result.isError).toBe(true);
    expect(result.content[0].text).toMatch(/An error occurred \(code: GEN-ERR-\d{3}\)/);
    expect(consoleErrorSpy).toHaveBeenCalled();
  });

  it('should use VALIDATION-001 code for ErrorCategory.VALIDATION', () => {
    const error = new Error('Validation failed');
    const result = logAndFormatError('testFunction', error, ErrorCategory.VALIDATION);

    expect(result.isError).toBe(true);
    expect(result.content[0].text).toBe(
      'An error occurred (code: VALIDATION-001). Check logs for details.'
    );
    expect(consoleErrorSpy).toHaveBeenCalledWith(
      expect.stringContaining('Code: VALIDATION-001'),
      error
    );
  });

  it('should use category-specific error code', () => {
    const error = new Error('Chat not found');
    const result = logAndFormatError('getChatInfo', error, ErrorCategory.CHAT);

    expect(result.isError).toBe(true);
    expect(result.content[0].text).toMatch(/An error occurred \(code: CHAT-ERR-\d{3}\)/);
    expect(consoleErrorSpy).toHaveBeenCalledWith(expect.stringContaining('CHAT-ERR-'), error);
  });

  it('should include context in console log', () => {
    const error = new Error('Test error');
    const context = { userId: 123, chatId: '456' };

    logAndFormatError('sendMessage', error, ErrorCategory.MSG, context);

    expect(consoleErrorSpy).toHaveBeenCalledWith(
      expect.stringContaining('userId=123, chatId=456'),
      error
    );
  });

  it('should work without category parameter', () => {
    const error = new Error('Generic error');
    const result = logAndFormatError('genericFunction', error);

    expect(result.isError).toBe(true);
    expect(result.content[0].text).toMatch(/An error occurred \(code: GEN-ERR-\d{3}\)/);
  });

  it('should work without context parameter', () => {
    const error = new Error('Test error');
    const result = logAndFormatError('testFunction', error, ErrorCategory.CONTACT);

    expect(result.isError).toBe(true);
    expect(consoleErrorSpy).toHaveBeenCalledWith(
      expect.stringContaining('[MCP] Error in testFunction ()'),
      error
    );
  });

  it('should handle different error categories', () => {
    const categories = [
      ErrorCategory.CHAT,
      ErrorCategory.MSG,
      ErrorCategory.CONTACT,
      ErrorCategory.GROUP,
      ErrorCategory.MEDIA,
      ErrorCategory.PROFILE,
      ErrorCategory.AUTH,
      ErrorCategory.ADMIN,
      ErrorCategory.SEARCH,
      ErrorCategory.DRAFT,
    ];

    categories.forEach(category => {
      const error = new Error('Test');
      const result = logAndFormatError('test', error, category);

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toMatch(
        new RegExp(`An error occurred \\(code: ${category}-ERR-\\d{3}\\)`)
      );
    });
  });
});

describe('withErrorHandling', () => {
  let consoleErrorSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
  });

  afterEach(() => {
    consoleErrorSpy.mockRestore();
  });

  it('should return result when async function succeeds', async () => {
    const successResult: MCPToolResult = { content: [{ type: 'text', text: 'Success' }] };

    const fn = async () => successResult;
    const wrapped = withErrorHandling(fn);
    const result = await wrapped();

    expect(result).toEqual(successResult);
    expect(consoleErrorSpy).not.toHaveBeenCalled();
  });

  it('should catch and format errors from async function', async () => {
    const fn = async () => {
      throw new Error('Function failed');
    };

    const wrapped = withErrorHandling(fn as any, ErrorCategory.CHAT);
    const result: any = await wrapped();

    expect(result.isError).toBe(true);
    expect(result.content[0].text).toMatch(/An error occurred \(code: CHAT-ERR-\d{3}\)/);
    expect(consoleErrorSpy).toHaveBeenCalled();
  });

  it('should catch ValidationError and pass through message', async () => {
    const fn = async () => {
      throw new ValidationError('Invalid input parameter');
    };

    const wrapped = withErrorHandling(fn as any);
    const result: any = await wrapped();

    expect(result.isError).toBe(true);
    expect(result.content[0].text).toBe('Invalid input parameter');
  });

  it('should handle non-Error throws by converting to Error', async () => {
    const fn = async () => {
      throw 'String error';
    };

    const wrapped = withErrorHandling(fn as any);
    const result: any = await wrapped();

    expect(result.isError).toBe(true);
    expect(result.content[0].text).toMatch(/An error occurred \(code: GEN-ERR-\d{3}\)/);
  });

  it('should preserve function arguments in error context', async () => {
    const fn = async (_chatId: number, _message: string) => {
      throw new Error('Test error');
    };

    const wrapped = withErrorHandling(fn as any, ErrorCategory.MSG);
    await wrapped(123, 'Hello');

    expect(consoleErrorSpy).toHaveBeenCalledWith(
      expect.stringContaining('args='),
      expect.any(Error)
    );
  });

  it('should use function name in error code generation', async () => {
    async function namedFunction() {
      throw new Error('Test');
    }

    const wrapped = withErrorHandling(namedFunction as any);
    await wrapped();

    expect(consoleErrorSpy).toHaveBeenCalledWith(
      expect.stringContaining('[MCP] Error in namedFunction'),
      expect.any(Error)
    );
  });

  it("should handle anonymous functions with 'unknown' name", async () => {
    const wrapped = withErrorHandling(
      (() => {
        const f = async () => {
          throw new Error('Test');
        };
        Object.defineProperty(f, 'name', { value: '' });
        return f;
      })() as any
    );

    await wrapped();

    expect(consoleErrorSpy).toHaveBeenCalledWith(
      expect.stringContaining('[MCP] Error in unknown'),
      expect.any(Error)
    );
  });
});
