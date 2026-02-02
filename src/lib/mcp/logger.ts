/**
 * MCP logger - simple console logger with [MCP] prefix
 */

type LogLevel = 'log' | 'warn' | 'error';

const PREFIX = '[MCP]';

function log(level: LogLevel, message: string, ...data: unknown[]): void {
  const fn = level === 'error' ? console.error : level === 'warn' ? console.warn : console.log;
  fn(PREFIX, message, ...data);
}

export function mcpLog(message: string, ...data: unknown[]): void {
  log('log', message, ...data);
}

export function mcpWarn(message: string, ...data: unknown[]): void {
  log('warn', message, ...data);
}

export function mcpError(message: string, ...data: unknown[]): void {
  log('error', message, ...data);
}
