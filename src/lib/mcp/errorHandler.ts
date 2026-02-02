/**
 * Error handling utilities for MCP server
 */
import type { MCPToolResult } from './types';
import { ValidationError } from './validation';

export enum ErrorCategory {
  CHAT = 'CHAT',
  MSG = 'MSG',
  CONTACT = 'CONTACT',
  GROUP = 'GROUP',
  MEDIA = 'MEDIA',
  PROFILE = 'PROFILE',
  AUTH = 'AUTH',
  ADMIN = 'ADMIN',
  VALIDATION = 'VALIDATION',
  SEARCH = 'SEARCH',
  DRAFT = 'DRAFT',
}

function generateErrorCode(functionName: string, category?: ErrorCategory | string): string {
  if (category === 'VALIDATION-001' || category === ErrorCategory.VALIDATION) {
    return 'VALIDATION-001';
  }

  const prefix = category
    ? typeof category === 'string' && category.startsWith('VALIDATION')
      ? category
      : category
    : 'GEN';

  const hash =
    Math.abs(functionName.split('').reduce((acc, char) => acc + char.charCodeAt(0), 0)) % 1000;

  return `${prefix}-ERR-${hash.toString().padStart(3, '0')}`;
}

export function logAndFormatError(
  functionName: string,
  error: Error,
  category?: ErrorCategory | string,
  context?: Record<string, unknown>
): MCPToolResult {
  const errorCode = generateErrorCode(functionName, category);
  const contextStr = context
    ? Object.entries(context)
        .map(([k, v]) => `${k}=${String(v)}`)
        .join(', ')
    : '';

  console.error(`[MCP] Error in ${functionName} (${contextStr}) - Code: ${errorCode}`, error);

  const userMessage =
    error instanceof ValidationError
      ? error.message
      : `An error occurred (code: ${errorCode}). Check logs for details.`;

  return { content: [{ type: 'text', text: userMessage }], isError: true };
}

export function withErrorHandling<T extends (...args: unknown[]) => Promise<MCPToolResult>>(
  fn: T,
  category?: ErrorCategory
): T {
  return (async (...args: Parameters<T>): Promise<MCPToolResult> => {
    try {
      return await fn(...args);
    } catch (error) {
      const functionName = fn.name || 'unknown';
      return logAndFormatError(
        functionName,
        error instanceof Error ? error : new Error(String(error)),
        category,
        { args: JSON.stringify(args) }
      );
    }
  }) as T;
}
