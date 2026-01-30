# MCP System

The Model Context Protocol (MCP) system enables AI-driven Telegram interactions through 81 specialized tools.

## Overview

```
MCP System Architecture
├── lib/mcp/
│   ├── index.ts              # Singleton lifecycle management
│   ├── types.ts              # MCP interfaces
│   ├── transport.ts          # Socket.IO JSON-RPC 2.0 transport
│   ├── logger.ts             # Logging utilities
│   ├── errorHandler.ts       # Error handling
│   ├── validation.ts         # Input validation
│   │
│   └── telegram/
│       ├── index.ts          # Server initialization
│       ├── server.ts         # TelegramMCPServer (81 tools)
│       ├── types.ts          # Tool name types
│       ├── telegramApi.ts    # Telegram API layer
│       ├── apiCastHelpers.ts # Type casting
│       ├── apiResultTypes.ts # Result types
│       ├── args.ts           # Argument schemas
│       ├── toolActionParser.ts # Human-readable parsing
│       │
│       └── tools/            # 81 individual tool files
```

## Core Components

### MCP Types (`lib/mcp/types.ts`)

```typescript
interface MCPTool {
  name: string;
  description: string;
  inputSchema: JSONSchema;
}

interface MCPRequest {
  jsonrpc: '2.0';
  id: string | number;
  method: string;
  params?: unknown;
}

interface MCPResponse {
  jsonrpc: '2.0';
  id: string | number;
  result?: unknown;
  error?: MCPError;
}

interface MCPError {
  code: number;
  message: string;
  data?: unknown;
}
```

### Transport Layer (`lib/mcp/transport.ts`)

Socket.IO-based JSON-RPC 2.0 transport.

**Features:**
- Request ID tracking
- 30-second timeout
- Error handling
- Event emission

```typescript
class SocketIOMCPTransport {
  constructor(socket: Socket);

  // Send request and await response
  async send(request: MCPRequest): Promise<MCPResponse>;

  // Register handler for incoming requests
  onRequest(handler: (req: MCPRequest) => Promise<MCPResponse>): void;

  // Clean up listeners
  dispose(): void;
}
```

### MCP Singleton (`lib/mcp/index.ts`)

Lifecycle management for MCP server.

```typescript
// Initialize MCP server with socket
initMCPServer(socket: Socket): TelegramMCPServer;

// Get existing server instance
getMCPServer(): TelegramMCPServer | null;

// Update socket reference
updateMCPSocket(socket: Socket): void;

// Clean up
cleanupMCP(): void;
```

## Telegram MCP Server

### Server Class (`lib/mcp/telegram/server.ts`)

```typescript
class TelegramMCPServer {
  constructor(transport: SocketIOMCPTransport, userId: string);

  // Register all 81 tools
  async initialize(): Promise<void>;

  // List available tools
  listTools(): MCPTool[];

  // Execute a tool
  async executeTool(name: string, args: unknown): Promise<ToolResult>;

  // Handle incoming JSON-RPC request
  async handleRequest(request: MCPRequest): Promise<MCPResponse>;

  // Clean up
  dispose(): void;
}
```

### Tool Handler Interface

Each tool file exports a handler:

```typescript
interface TelegramMCPToolHandler {
  name: string;
  description: string;
  inputSchema: JSONSchema;
  call: (
    args: unknown,
    context: {
      telegramClient: TelegramClient;
      userId: string;
    }
  ) => Promise<ToolResult>;
}

interface ToolResult {
  success: boolean;
  data?: unknown;
  error?: string;
}
```

## Tool Categories

### 81 Telegram Tools

#### User & Profile (5 tools)
| Tool | Description |
|------|-------------|
| `getMe` | Get current authenticated user |
| `getUserInfo` | Get info about any user |
| `getUserPhotos` | Get user profile photos |
| `getUserStatus` | Get online status |
| `setUserStatus` | Update own status |

#### Chats & Dialogs (12 tools)
| Tool | Description |
|------|-------------|
| `getChats` | Fetch chat list |
| `getChatInfo` | Get chat details |
| `createGroup` | Create new group |
| `createChannel` | Create new channel |
| `leaveChat` | Leave chat/group/channel |
| `deleteChat` | Delete chat |
| `muteChat` | Mute notifications |
| `unmuteChat` | Unmute notifications |
| `pinChat` | Pin chat to top |
| `unpinChat` | Unpin chat |
| `archiveChat` | Archive chat |
| `unarchiveChat` | Unarchive chat |

#### Messages (15 tools)
| Tool | Description |
|------|-------------|
| `getHistory` | Fetch message history |
| `sendMessage` | Send text message |
| `replyToMessage` | Reply to specific message |
| `editMessage` | Edit sent message |
| `deleteMessage` | Delete message |
| `forwardMessage` | Forward to another chat |
| `pinMessage` | Pin message in chat |
| `unpinMessage` | Unpin message |
| `markAsRead` | Mark messages as read |
| `searchMessages` | Search in chat |
| `translateMessage` | Translate message text |
| `getMessageReactions` | Get reactions |
| `addReaction` | React to message |
| `removeReaction` | Remove reaction |
| `reportMessage` | Report spam/abuse |

#### Media (10 tools)
| Tool | Description |
|------|-------------|
| `sendPhoto` | Send image |
| `sendVideo` | Send video |
| `sendDocument` | Send file |
| `sendVoice` | Send voice message |
| `sendAudio` | Send audio file |
| `sendSticker` | Send sticker |
| `sendGif` | Send animation |
| `sendLocation` | Send location |
| `sendContact` | Send contact card |
| `downloadMedia` | Download media file |

#### Contacts (8 tools)
| Tool | Description |
|------|-------------|
| `addContact` | Add new contact |
| `deleteContact` | Remove contact |
| `getContacts` | Get contact list |
| `searchContacts` | Search contacts |
| `importContacts` | Bulk import |
| `exportContacts` | Export contact list |
| `blockUser` | Block user |
| `unblockUser` | Unblock user |

#### Groups & Channels (15 tools)
| Tool | Description |
|------|-------------|
| `getAdmins` | Get admin list |
| `getMembers` | Get member list |
| `addMember` | Add user to group |
| `removeMember` | Remove from group |
| `banUser` | Ban user |
| `unbanUser` | Unban user |
| `promoteAdmin` | Promote to admin |
| `demoteAdmin` | Remove admin rights |
| `setGroupTitle` | Change group name |
| `setGroupPhoto` | Change group photo |
| `setGroupDescription` | Change description |
| `subscribePublicChannel` | Join public channel |
| `inviteToChannel` | Invite user |
| `getInviteLink` | Get invite link |
| `revokeInviteLink` | Revoke invite link |

#### Polls & Interactive (5 tools)
| Tool | Description |
|------|-------------|
| `createPoll` | Create poll |
| `votePoll` | Vote on poll |
| `stopPoll` | Close poll |
| `pressInlineButton` | Click inline button |
| `answerCallback` | Respond to callback |

#### Drafts (3 tools)
| Tool | Description |
|------|-------------|
| `saveDraft` | Save draft message |
| `getDrafts` | Get all drafts |
| `deleteDraft` | Delete draft |

#### Privacy & Settings (5 tools)
| Tool | Description |
|------|-------------|
| `getBlockedUsers` | Get blocked list |
| `getPrivacySettings` | Get privacy config |
| `updatePrivacy` | Update privacy |
| `get2FAStatus` | Check 2FA status |
| `getActiveSessions` | Get login sessions |

#### Misc (3 tools)
| Tool | Description |
|------|-------------|
| `resolveUsername` | Resolve @username |
| `checkUsername` | Check availability |
| `getWebPage` | Get link preview |

## Tool Implementation Example

```typescript
// lib/mcp/telegram/tools/sendMessage.ts
import { TelegramMCPToolHandler } from '../types';

export const sendMessage: TelegramMCPToolHandler = {
  name: 'sendMessage',
  description: 'Send a text message to a chat',
  inputSchema: {
    type: 'object',
    properties: {
      chatId: {
        type: 'string',
        description: 'Chat ID to send message to'
      },
      text: {
        type: 'string',
        description: 'Message text'
      },
      replyToMsgId: {
        type: 'string',
        description: 'Optional message ID to reply to'
      }
    },
    required: ['chatId', 'text']
  },

  call: async (args, { telegramClient, userId }) => {
    const { chatId, text, replyToMsgId } = args as {
      chatId: string;
      text: string;
      replyToMsgId?: string;
    };

    try {
      // Use big-integer for chat ID (Telegram IDs exceed Number.MAX_SAFE_INTEGER)
      const peer = await telegramClient.getEntity(bigInt(chatId));

      const result = await telegramClient.sendMessage(peer, {
        message: text,
        replyTo: replyToMsgId ? parseInt(replyToMsgId) : undefined
      });

      return {
        success: true,
        data: {
          messageId: result.id.toString(),
          date: result.date
        }
      };
    } catch (error) {
      return {
        success: false,
        error: error.message
      };
    }
  }
};
```

## MCP Initialization Flow

```typescript
// In SocketProvider.tsx
useEffect(() => {
  if (socket && socket.connected) {
    // Initialize MCP server
    const mcpServer = initMCPServer(socket);

    // Server registers all 81 tools
    mcpServer.initialize();

    // Listen for tool execution requests
    mcpServer.onRequest(async (request) => {
      if (request.method === 'tools/call') {
        const { name, arguments: args } = request.params;
        return mcpServer.executeTool(name, args);
      }
      return { error: { code: -32601, message: 'Method not found' } };
    });
  }

  return () => cleanupMCP();
}, [socket?.connected]);
```

## JSON-RPC Protocol

### Request Format
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "sendMessage",
    "arguments": {
      "chatId": "123456789",
      "text": "Hello from AI!"
    }
  }
}
```

### Response Format (Success)
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "success": true,
    "data": {
      "messageId": "12345",
      "date": 1706540000
    }
  }
}
```

### Response Format (Error)
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "error": {
    "code": -32000,
    "message": "Chat not found",
    "data": { "chatId": "invalid" }
  }
}
```

## Big Integer Handling

Telegram IDs exceed JavaScript's `Number.MAX_SAFE_INTEGER`. Use `big-integer` library:

```typescript
import bigInt from 'big-integer';

// Convert string ID to big integer
const chatId = bigInt(args.chatId);

// Convert big integer back to string
const idString = chatId.toString();
```

## Best Practices

1. **Use big-integer for IDs** - All Telegram IDs should use big-integer
2. **Validate inputs** - Use JSON Schema validation before execution
3. **Handle errors gracefully** - Return structured error responses
4. **Log operations** - Use MCP logger for debugging
5. **Timeout long operations** - 30s timeout for transport

---

*Previous: [Services Layer](./03-services.md) | Next: [Pages & Routing](./05-pages-routing.md)*
