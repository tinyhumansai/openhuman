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
  private eventHandlers = new Map<string, Map<Function, Function>>();
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
    // If the socket drops while a request is in flight, the response will
    // never arrive and the caller would otherwise block until the 30s
    // request timeout. Drain pending handlers immediately so callers see
    // a clear `Socket disconnected` error and can recover / retry.
    this.socket.on('disconnect', this.disconnectHandler);
  }

  private disconnectHandler = (reason?: string): void => {
    this.rejectAllPending(`Socket disconnected${reason ? `: ${reason}` : ''}`);
  };

  /**
   * Fail every in-flight request with a synthetic JSON-RPC error so its
   * promise rejects instead of leaking into the 30s request timeout.
   * Used on socket `disconnect` and when `updateSocket` replaces the
   * underlying transport (since old in-flight requests were emitted on the
   * previous socket and can never receive a response on the new one).
   */
  private rejectAllPending(reason: string): void {
    if (this.requestHandlers.size === 0) return;
    const handlers = Array.from(this.requestHandlers.entries());
    this.requestHandlers.clear();
    mcpWarn('Rejecting pending MCP requests', { count: handlers.length, reason });
    for (const [id, handler] of handlers) {
      handler({ jsonrpc: '2.0', id, error: { code: -32000, message: reason } });
    }
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

    let handlersForEvent = this.eventHandlers.get(fullEvent);
    if (!handlersForEvent) {
      handlersForEvent = new Map();
      this.eventHandlers.set(fullEvent, handlersForEvent);
    }
    handlersForEvent.set(handler, wrappedHandler);

    this.socket.on(fullEvent, wrappedHandler);
  }

  off(event: string, handler: (data: unknown) => void): void {
    if (!this.socket) return;
    const fullEvent = `${this.eventPrefix}${event}`;
    const handlersForEvent = this.eventHandlers.get(fullEvent);
    const wrappedHandler = handlersForEvent?.get(handler);

    if (wrappedHandler) {
      this.socket.off(fullEvent, wrappedHandler as any);
      handlersForEvent?.delete(handler);
    } else {
      this.socket.off(fullEvent, handler);
    }
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
      this.socket.off('disconnect', this.disconnectHandler);
    }
    // Pending handlers were emitted on the old socket; the new socket will
    // never deliver their responses, so reject them now rather than letting
    // them hang until the per-request timeout fires.
    this.rejectAllPending('Socket replaced');
    this.socket = socket ?? undefined;
    this.setupEventHandlers();
  }
}
