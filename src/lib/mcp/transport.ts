/**
 * Socket.IO transport for MCP
 * Handles communication between frontend MCP server and backend MCP client
 */
import type { Socket } from 'socket.io-client';

import { createSafeLogData, sanitizeError } from '../../utils/sanitize';
import { mcpError, mcpLog, mcpWarn } from './logger';
import type { MCPRequest, MCPResponse, SocketIOMCPTransport } from './types';

export class SocketIOMCPTransportImpl implements SocketIOMCPTransport {
  private socket: Socket | null | undefined;
  private requestHandlers = new Map<string | number, (response: MCPResponse) => void>();
  private readonly eventPrefix = 'mcp:';
  private responseHandler = (response: MCPResponse): void => {
    mcpLog(
      'Received response',
      createSafeLogData(
        { id: response.id, hasError: !!response.error, hasResult: !!response.result },
        response
      )
    );
    const handler = this.requestHandlers.get(response.id);
    if (handler) {
      handler(response);
      this.requestHandlers.delete(response.id);
    } else {
      mcpWarn('No handler found for response', { id: response.id });
    }
  };

  constructor(socket: Socket | null | undefined) {
    this.socket = socket ?? undefined;
    this.setupEventHandlers();
  }

  get connected(): boolean {
    return Boolean(this.socket?.connected);
  }

  private setupEventHandlers(): void {
    if (!this.socket) return;
    this.socket.on(`${this.eventPrefix}response`, this.responseHandler);
  }

  emit(event: string, data: unknown): void {
    if (!this.socket?.connected) {
      mcpWarn('Cannot emit MCP event: socket not connected', { event });
      return;
    }
    const fullEvent = `${this.eventPrefix}${event}`;
    mcpLog('Emitting event', createSafeLogData({ event: fullEvent }, data));
    this.socket.emit(fullEvent, data);
  }

  on(event: string, handler: (data: unknown) => void): void {
    if (!this.socket) return;
    const fullEvent = `${this.eventPrefix}${event}`;
    const wrappedHandler = (data: unknown) => {
      mcpLog('Received event', createSafeLogData({ event: fullEvent }, data));
      handler(data);
    };
    this.socket.on(fullEvent, wrappedHandler);
  }

  off(event: string, handler: (data: unknown) => void): void {
    if (!this.socket) return;
    this.socket.off(`${this.eventPrefix}${event}`, handler);
  }

  async request(request: MCPRequest, timeoutMs = 30000): Promise<MCPResponse> {
    if (!this.socket?.connected) {
      throw new Error('Socket not connected');
    }

    mcpLog('Sending request', { id: request.id, method: request.method, timeoutMs });

    return new Promise<MCPResponse>((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.requestHandlers.delete(request.id);
        mcpError('Request timeout', { id: request.id, method: request.method, timeoutMs });
        reject(new Error(`MCP request timeout after ${timeoutMs}ms`));
      }, timeoutMs);

      this.requestHandlers.set(request.id, (response: MCPResponse) => {
        clearTimeout(timeout);
        if (response.error) {
          mcpError('Request error', {
            id: request.id,
            method: request.method,
            error: sanitizeError(response.error),
          });
          reject(new Error(response.error.message));
        } else {
          mcpLog('Request success', { id: request.id, method: request.method });
          resolve(response);
        }
      });

      this.emit('request', request);
    });
  }

  updateSocket(socket: Socket | null | undefined): void {
    if (this.socket) {
      this.socket.off(`${this.eventPrefix}response`, this.responseHandler);
    }
    this.socket = socket ?? undefined;
    this.setupEventHandlers();
  }
}
