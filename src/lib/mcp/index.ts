/**
 * MCP (Model Context Protocol) shared layer
 * Used by MCP servers (e.g. telegram, gmail, etc.)
 */

export * from './types';
export * from './validation';
export * from './errorHandler';
export * from './logger';
export { SocketIOMCPTransportImpl } from './transport';
export {
  enforceRateLimit,
  resetRequestCallCount,
  classifyTool,
  isStateOnlyTool,
  isReadOnlyTool,
  isHeavyTool,
  getRateLimitStatus,
  RATE_LIMIT_CONFIG,
} from './rateLimiter';
export type { ToolTier } from './rateLimiter';
