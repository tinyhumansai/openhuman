import debug from 'debug';
import { io, Socket } from 'socket.io-client';

import { MCPTool, MCPToolCall, SocketIOMCPTransportImpl } from '../lib/mcp';
import { skillManager } from '../lib/skills';
import { store } from '../store';
import { resetForUser, setSocketIdForUser, setStatusForUser } from '../store/socketSlice';
import { BACKEND_URL } from '../utils/config';
import { createSafeLogData, sanitizeError } from '../utils/sanitize';

// Socket service logger using debug package
// Enable logging by setting DEBUG=socket* in environment or localStorage
const socketLog = debug('socket');
const socketWarn = debug('socket:warn');
const socketError = debug('socket:error');

// Enable socket logging in development by default
if (import.meta.env.DEV || import.meta.env.MODE === 'development') {
  debug.enable('socket*');
}

interface JwtPayload {
  tgUserId?: string;
  userId?: string;
  sub?: string;
}

function getSocketUserId(): string {
  const token = store.getState().auth.token;
  if (!token) return '__pending__';

  try {
    const parts = token.split('.');
    if (parts.length !== 3) return '__pending__';

    const payloadBase64 = parts[1].replace(/-/g, '+').replace(/_/g, '/');
    const payloadJson = atob(payloadBase64);
    const payload = JSON.parse(payloadJson) as JwtPayload;

    const id = payload.tgUserId || payload.userId || payload.sub;
    return id || '__pending__';
  } catch {
    return '__pending__';
  }
}

class SocketService {
  private socket: Socket | null = null;
  private token: string | null = null;
  private mcpTransport: SocketIOMCPTransportImpl | null = null;

  /**
   * Connect to the socket server with authentication
   */
  connect(token: string): void {
    if (!token) return;

    // Don't connect if already connected with the same token
    if (this.socket?.connected && this.token === token) return;

    // Disconnect existing connection if token changed or socket exists
    if (this.socket) {
      if (this.token !== token) {
        this.disconnect();
      } else if (this.socket.connected) {
        return;
      } else if (!this.socket.disconnected) {
        // Socket is connecting, wait for it
        return;
      }
    }

    this.token = token;
    const uid = getSocketUserId();

    socketLog('Connecting', { userId: uid, backendUrl: BACKEND_URL });

    store.dispatch(setStatusForUser({ userId: uid, status: 'connecting' }));

    const backendUrl = BACKEND_URL;

    // Ensure we're not connecting to the wrong URL
    if (backendUrl.includes('localhost:1420') || backendUrl.includes(':1420')) {
      return;
    }

    // Create socket connection with auth token
    // Note: path must match backend server configuration
    // Backend expects token in socket.handshake.auth.token (NOT in query string)
    // Match the working test script configuration: start with polling, then upgrade
    // Socket.io sends auth in the handshake (POST request body for polling, not in GET headers)
    const socketOptions = {
      auth: { token },
      path: '/socket.io/',
      transports: ['websocket', 'polling'], // Start with polling (more reliable), then upgrade to websocket
      reconnection: true,
      reconnectionDelay: 1000,
      reconnectionAttempts: 5,
      forceNew: true, // Force new connection to ensure auth is sent
      timeout: 2000, // Increase timeout for initial connection
      upgrade: true, // Allow upgrade from polling to websocket
      query: {}, // Explicitly prevent token from being added to query string
    };

    this.socket = io(backendUrl, socketOptions);

    // Initialize MCP transport for client→server MCP requests
    this.mcpTransport = new SocketIOMCPTransportImpl(this.socket);

    // Connection event handlers
    this.socket.on('connect', () => {
      const socketId = this.socket?.id || null;
      const uid = getSocketUserId();
      socketLog('Connected', { socketId, userId: uid });
      store.dispatch(setStatusForUser({ userId: uid, status: 'connected' }));
      store.dispatch(setSocketIdForUser({ userId: uid, socketId }));
    });

    this.socket.on('ready', () => {
      const uid = getSocketUserId();
      socketLog('Server ready - authentication successful', { userId: uid });
    });

    this.socket.on('error', (error: unknown) => {
      const uid = getSocketUserId();
      socketError('Server error', { userId: uid, error: sanitizeError(error) });
    });

    this.socket.on('disconnect', (reason: string) => {
      const uid = getSocketUserId();
      socketLog('Disconnected', { userId: uid, reason });
      store.dispatch(setStatusForUser({ userId: uid, status: 'disconnected' }));
      store.dispatch(setSocketIdForUser({ userId: uid, socketId: null }));
    });

    this.socket.on('connect_error', (error: Error) => {
      const uid = getSocketUserId();
      socketError('Connection error', { userId: uid, error: sanitizeError(error) });
      store.dispatch(setStatusForUser({ userId: uid, status: 'disconnected' }));
    });

    this.socket.on('mcp:listTools', (data: { requestId: string }) => {
      socketLog('MCP list tools request', { requestId: data.requestId });

      // Aggregate tools from all ready skills
      const skillsState = store.getState().skills.skills;
      const allTools: MCPTool[] = [];

      for (const [skillId, skill] of Object.entries(skillsState)) {
        if (skill.status === 'ready' && skill.tools?.length) {
          for (const tool of skill.tools) {
            allTools.push({
              name: `${skillId}__${tool.name}`,
              description: tool.description,
              inputSchema: tool.inputSchema,
            });
          }
        }
      }

      socketLog('MCP list tools response', {
        requestId: data.requestId,
        toolCount: allTools.length,
      });

      this.socket?.emit('mcp:listToolsResponse', { requestId: data.requestId, tools: allTools });
    });

    this.socket.on('mcp:toolCall', async (data: { requestId: string; toolCall: MCPToolCall }) => {
      const { requestId, toolCall } = data;
      socketLog('MCP tool call', createSafeLogData({ requestId, toolName: toolCall?.name }, data));

      // Tool names are namespaced as "skillId__toolName" (double underscore)
      // Skill names cannot contain underscores to avoid ambiguity
      const separatorIdx = toolCall.name.indexOf('__');
      if (separatorIdx === -1) {
        socketError('MCP tool call - invalid tool name format', { requestId, name: toolCall.name });
        this.socket?.emit('mcp:toolCallResponse', {
          requestId,
          result: {
            content: [
              {
                type: 'text',
                text: `Invalid tool name: ${toolCall.name}. Expected format: skillId__toolName`,
              },
            ],
            isError: true,
          },
        });
        return;
      }

      const skillId = toolCall.name.substring(0, separatorIdx);
      const toolName = toolCall.name.substring(separatorIdx + 2);

      try {
        const result = await skillManager.callTool(skillId, toolName, toolCall.arguments);

        socketLog('MCP tool call success', { requestId, skillId, toolName });

        this.socket?.emit('mcp:toolCallResponse', { requestId, result });
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        socketError('MCP tool call failed', {
          requestId,
          skillId,
          toolName,
          error: sanitizeError(err),
        });
        this.socket?.emit('mcp:toolCallResponse', {
          requestId,
          result: { content: [{ type: 'text', text: msg }], isError: true },
        });
      }
    });

    this.socket.connect();
  }

  /**
   * Disconnect from the socket server
   */
  disconnect(): void {
    if (this.socket) {
      const uid = getSocketUserId();
      socketLog('Disconnecting', { userId: uid });
      this.socket.disconnect();
      this.socket = null;
      this.token = null;
      this.mcpTransport = null;
      store.dispatch(resetForUser({ userId: uid }));
    }
  }

  /**
   * Get the current socket instance
   */
  getSocket(): Socket | null {
    return this.socket;
  }

  /**
   * Get the MCP transport for making client→server MCP requests
   */
  getMCPTransport(): SocketIOMCPTransportImpl | null {
    return this.mcpTransport;
  }

  /**
   * Check if socket is connected
   */
  isConnected(): boolean {
    return this.socket?.connected || false;
  }

  /**
   * Emit an event to the server
   */
  emit(event: string, data?: unknown): void {
    if (this.socket?.connected) {
      socketLog('Emitting event', createSafeLogData({ event }, data));
      this.socket.emit(event, data);
    } else {
      socketWarn('Cannot emit event - socket not connected', { event });
    }
  }

  /**
   * Listen to an event from the server
   */
  on(event: string, callback: (...args: unknown[]) => void): void {
    if (this.socket) {
      const wrappedCallback = (...args: unknown[]) => {
        socketLog('Received event', { event, argsCount: args.length, hasData: args.length > 0 });
        callback(...args);
      };
      this.socket.on(event, wrappedCallback);
    }
  }

  /**
   * Remove an event listener
   */
  off(event: string, callback?: (...args: unknown[]) => void): void {
    if (this.socket) {
      if (callback) {
        this.socket.off(event, callback);
      } else {
        this.socket.off(event);
      }
    }
  }

  /**
   * Listen to an event once
   */
  once(event: string, callback: (...args: unknown[]) => void): void {
    if (this.socket) {
      const wrappedCallback = (...args: unknown[]) => {
        socketLog('Received event (once)', {
          event,
          argsCount: args.length,
          hasData: args.length > 0,
        });
        callback(...args);
      };
      this.socket.once(event, wrappedCallback);
    }
  }
}

export const socketService = new SocketService();
