/**
 * E2E integration tests for TelegramMCPServer
 *
 * Tests the full pipeline: transport event → server → tool handler → transport emit
 */

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import type { MCPToolResult } from "../../mcp/types";

// Use vi.hoisted so mock variables are available when vi.mock factories run
const {
  mockTransportEmit,
  mockTransportOn,
  mockTransportOff,
  mockTransportUpdateSocket,
} = vi.hoisted(() => ({
  mockTransportEmit: vi.fn(),
  mockTransportOn: vi.fn(),
  mockTransportOff: vi.fn(),
  mockTransportUpdateSocket: vi.fn(),
}));

// Mock the transport module — must use hoisted variables
vi.mock("../../mcp/transport", () => ({
  SocketIOMCPTransportImpl: vi.fn(),
}));

// Mock the store
vi.mock("../../../store", () => ({
  store: {
    getState: vi.fn(() => ({
      user: { user: { _id: "u1" } },
      telegram: {
        byUser: {
          u1: {
            connectionStatus: "connected",
            authStatus: "authenticated",
            isInitialized: true,
            chats: {},
            chatsOrder: [],
            messages: {},
            messagesOrder: {},
            currentUser: null,
            users: {},
            selectedChatId: null,
            threads: {},
            threadsOrder: {},
            selectedThreadId: null,
            threadIndex: {},
            isLoadingChats: false,
            isLoadingMessages: false,
            isLoadingThreads: false,
            hasMoreChats: true,
            hasMoreMessages: {},
            hasMoreThreads: {},
            searchQuery: null,
            filteredChatIds: null,
            connectionError: null,
            authError: null,
            phoneNumber: null,
            sessionString: null,
            isSyncing: false,
            isSynced: false,
            commonBoxState: { seq: 0, date: 0, pts: 0, qts: 0 },
            channelPtsById: {},
          },
        },
      },
    })),
  },
}));

vi.mock("../../../store/telegramSelectors", () => ({
  selectTelegramUserState: vi.fn((state: any) => {
    const userId = state?.user?.user?._id ?? "";
    return state?.telegram?.byUser?.[userId] ?? {};
  }),
  selectOrderedChats: vi.fn(() => []),
  selectCurrentUser: vi.fn(() => null),
}));

// Mock the skills module
vi.mock("../../mcp/skills", () => ({
  useExtraToolDefinition: {
    name: "use_extra_tool",
    description: "meta",
    inputSchema: { type: "object", properties: {} },
  },
  executeUseExtraTool: vi.fn(),
  executeExtraToolIfExists: vi.fn(() => null),
  getAllExtraTools: vi.fn(() => []),
  isExtraToolByName: vi.fn(() => false),
}));

// Mock rateLimiter
vi.mock("../../mcp/rateLimiter", () => ({
  enforceRateLimit: vi.fn().mockResolvedValue(undefined),
  resetRequestCallCount: vi.fn(),
  isStateOnlyTool: vi.fn(() => false),
  classifyTool: vi.fn(() => "api_read"),
  isHeavyTool: vi.fn(() => false),
}));

// Mock logger
vi.mock("../../mcp/logger", () => ({
  mcpLog: vi.fn(),
  mcpWarn: vi.fn(),
}));

// Mock API functions used by tool handlers
vi.mock("../api/getChats", () => ({
  getChats: vi.fn(() =>
    Promise.resolve({
      data: [
        {
          id: "1",
          title: "Chat One",
          type: "private",
          unreadCount: 0,
          isPinned: false,
        },
        {
          id: "2",
          title: "Group Chat",
          type: "group",
          unreadCount: 5,
          isPinned: true,
        },
      ],
      fromCache: true,
    })
  ),
}));

vi.mock("../api/sendMessage", () => ({
  sendMessage: vi.fn(() =>
    Promise.resolve({
      data: { id: "msg1", message: "Hello", chatId: "1", date: 1234567890 },
      fromCache: false,
    })
  ),
}));

vi.mock("../api/getCurrentUser", () => ({
  getCurrentUser: vi.fn(() =>
    Promise.resolve({
      data: {
        id: "1",
        firstName: "Alice",
        lastName: "Smith",
        username: "alice",
        isBot: false,
      },
      fromCache: true,
    })
  ),
}));

vi.mock("../api/getMessages", () => ({
  getMessages: vi.fn(() =>
    Promise.resolve({
      data: [
        {
          id: "m1",
          message: "First message",
          chatId: "1",
          date: 1234567890,
          fromId: "100",
        },
        {
          id: "m2",
          message: "Second message",
          chatId: "1",
          date: 1234567891,
          fromId: "200",
        },
      ],
      fromCache: true,
    })
  ),
}));

vi.mock("../api/helpers", async (importOriginal) => {
  const original = (await importOriginal()) as any;
  return {
    ...original,
    getChatById: vi.fn(() => ({
      id: "1",
      title: "Chat One",
      type: "private",
      unreadCount: 0,
      isPinned: false,
    })),
    getOrderedChats: vi.fn(() => []),
  };
});

// Helper: extract emitted data for a given event name
function getEmittedResult(eventName: string): MCPToolResult | undefined {
  const emitCall = mockTransportEmit.mock.calls.find(
    (call: any[]) => call[0] === eventName
  );
  return emitCall?.[1]?.result;
}

function getEmittedPayload(eventName: string): any {
  const emitCall = mockTransportEmit.mock.calls.find(
    (call: any[]) => call[0] === eventName
  );
  return emitCall?.[1];
}

describe("TelegramMCPServer", () => {
  let toolCallHandler: Function;
  let listToolsHandler: Function;

  beforeEach(async () => {
    vi.clearAllMocks();

    // Re-apply mock implementations (mockReset: true in config clears them between tests)
    const { SocketIOMCPTransportImpl } = await import("../../mcp/transport");
    vi.mocked(SocketIOMCPTransportImpl).mockImplementation(function (
      this: any
    ) {
      this.on = mockTransportOn;
      this.off = mockTransportOff;
      this.emit = mockTransportEmit;
      this.updateSocket = mockTransportUpdateSocket;
      this.connected = true;
      return this;
    } as any);

    const rateLimiter = await import("../../mcp/rateLimiter");
    vi.mocked(rateLimiter.enforceRateLimit).mockResolvedValue(undefined);
    vi.mocked(rateLimiter.resetRequestCallCount).mockImplementation(() => {});
    vi.mocked(rateLimiter.isStateOnlyTool).mockReturnValue(false);

    const skills = await import("../../mcp/skills");
    vi.mocked(skills.executeExtraToolIfExists).mockReturnValue(null as any);
    vi.mocked(skills.getAllExtraTools).mockReturnValue([]);
    vi.mocked(skills.isExtraToolByName).mockReturnValue(false);

    // Re-apply API mocks
    const getChatsApi = await import("../api/getChats");
    vi.mocked(getChatsApi.getChats).mockResolvedValue({
      data: [
        {
          id: "1",
          title: "Chat One",
          type: "private",
          unreadCount: 0,
          isPinned: false,
        },
        {
          id: "2",
          title: "Group Chat",
          type: "group",
          unreadCount: 5,
          isPinned: true,
        },
      ],
      fromCache: true,
    });

    const sendMsgApi = await import("../api/sendMessage");
    vi.mocked(sendMsgApi.sendMessage).mockResolvedValue({
      data: {
        id: "msg1",
        message: "Hello",
        chatId: "1",
        date: 1234567890,
      } as any,
      fromCache: false,
    });

    const getUserApi = await import("../api/getCurrentUser");
    vi.mocked(getUserApi.getCurrentUser).mockResolvedValue({
      data: {
        id: "1",
        firstName: "Alice",
        lastName: "Smith",
        username: "alice",
        isBot: false,
      } as any,
      fromCache: true,
    });

    const getMsgsApi = await import("../api/getMessages");
    vi.mocked(getMsgsApi.getMessages).mockResolvedValue({
      data: [
        {
          id: "m1",
          message: "First message",
          chatId: "1",
          date: 1234567890,
          fromId: "100",
        } as any,
        {
          id: "m2",
          message: "Second message",
          chatId: "1",
          date: 1234567891,
          fromId: "200",
        } as any,
      ],
      fromCache: true,
    });

    const helpers = await import("../api/helpers");
    vi.mocked(helpers.getChatById).mockReturnValue({
      id: "1",
      title: "Chat One",
      type: "private",
      unreadCount: 0,
      isPinned: false,
    });

    const store = await import("../../../store");
    vi.mocked(store.store.getState).mockReturnValue({
      user: { user: { _id: "u1" } },
      telegram: {
        byUser: {
          u1: {
            connectionStatus: "connected",
            authStatus: "authenticated",
            isInitialized: true,
            chats: {},
            chatsOrder: [],
            messages: {},
            messagesOrder: {},
            currentUser: null,
          },
        },
      },
    } as any);

    // Import server after all mocks are set up
    const { TelegramMCPServer } = await import("../server");

    // Create server instance — constructor calls setupHandlers() which calls
    // transport.on("toolCall", ...) and transport.on("listTools", ...)
    new TelegramMCPServer(null);

    // Extract the registered event handlers from transport.on calls
    const onCalls = mockTransportOn.mock.calls;
    const toolCallEntry = onCalls.find((call: any[]) => call[0] === "toolCall");
    const listToolsEntry = onCalls.find(
      (call: any[]) => call[0] === "listTools"
    );

    toolCallHandler = toolCallEntry![1];
    listToolsHandler = listToolsEntry![1];
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe("listTools", () => {
    it("should return array of available tools with correct shape", () => {
      listToolsHandler({ requestId: "list-req-1" });

      const payload = getEmittedPayload("listToolsResponse");
      expect(payload).toBeDefined();
      expect(payload.requestId).toBe("list-req-1");

      const tools = payload.tools;
      expect(Array.isArray(tools)).toBe(true);
      expect(tools.length).toBeGreaterThan(50);

      // Verify each tool has required properties
      tools.forEach((tool: any) => {
        expect(tool).toHaveProperty("name");
        expect(tool).toHaveProperty("description");
        expect(tool).toHaveProperty("inputSchema");
        expect(typeof tool.name).toBe("string");
        expect(typeof tool.description).toBe("string");
        expect(typeof tool.inputSchema).toBe("object");
      });
    });

    it("should include common tools like get_chats, send_message, get_me", () => {
      listToolsHandler({ requestId: "list-req-2" });

      const payload = getEmittedPayload("listToolsResponse");
      const toolNames = payload.tools.map((t: any) => t.name);

      expect(toolNames).toContain("get_chats");
      expect(toolNames).toContain("send_message");
      expect(toolNames).toContain("get_me");
      expect(toolNames).toContain("get_messages");
    });
  });

  describe("Tool execution - get_chats", () => {
    it("should successfully execute get_chats and return chat list", async () => {
      await toolCallHandler({
        requestId: "req-1",
        toolCall: { name: "get_chats", arguments: {} },
      });

      // Allow any microtasks to settle
      await vi.waitFor(() => {
        expect(mockTransportEmit).toHaveBeenCalledWith(
          "toolResult",
          expect.objectContaining({ requestId: "req-1" })
        );
      });

      const result = getEmittedResult("toolResult");
      expect(result).toBeDefined();
      expect(result!.isError).not.toBe(true);
      expect(result!.content).toBeDefined();
      expect(result!.content.length).toBeGreaterThan(0);

      const contentText = JSON.stringify(result!.content);
      expect(contentText).toContain("Chat One");
    });

    it("should execute get_chats with limit parameter", async () => {
      await toolCallHandler({
        requestId: "req-2",
        toolCall: { name: "get_chats", arguments: { page_size: 1 } },
      });

      await vi.waitFor(() => {
        expect(mockTransportEmit).toHaveBeenCalled();
      });

      const result = getEmittedResult("toolResult");
      expect(result).toBeDefined();
      expect(result!.isError).not.toBe(true);
    });
  });

  describe("Tool execution - send_message", () => {
    it("should successfully send a message", async () => {
      await toolCallHandler({
        requestId: "req-3",
        toolCall: {
          name: "send_message",
          arguments: { chat_id: "1", message: "Hello from test" },
        },
      });

      await vi.waitFor(() => {
        expect(mockTransportEmit).toHaveBeenCalled();
      });

      const result = getEmittedResult("toolResult");
      expect(result).toBeDefined();
      expect(result!.isError).not.toBe(true);
      expect(result!.content).toBeDefined();
    });

    it("should fail when chat_id is missing", async () => {
      await toolCallHandler({
        requestId: "req-4",
        toolCall: {
          name: "send_message",
          arguments: { message: "Hello without chat_id" },
        },
      });

      await vi.waitFor(() => {
        expect(mockTransportEmit).toHaveBeenCalled();
      });

      const result = getEmittedResult("toolResult");
      expect(result).toBeDefined();
      expect(result!.isError).toBe(true);
    });

    it("should fail when text is missing", async () => {
      await toolCallHandler({
        requestId: "req-5",
        toolCall: {
          name: "send_message",
          arguments: { chat_id: "1" },
        },
      });

      await vi.waitFor(() => {
        expect(mockTransportEmit).toHaveBeenCalled();
      });

      const result = getEmittedResult("toolResult");
      expect(result).toBeDefined();
      expect(result!.isError).toBe(true);
    });
  });

  describe("Tool execution - get_me", () => {
    it("should return current user information", async () => {
      await toolCallHandler({
        requestId: "req-6",
        toolCall: { name: "get_me", arguments: {} },
      });

      await vi.waitFor(() => {
        expect(mockTransportEmit).toHaveBeenCalled();
      });

      const result = getEmittedResult("toolResult");
      expect(result).toBeDefined();
      expect(result!.isError).not.toBe(true);
      expect(result!.content).toBeDefined();

      const contentText = JSON.stringify(result!.content);
      expect(contentText).toContain("Alice");
    });
  });

  describe("Tool execution - get_messages", () => {
    it("should successfully retrieve messages from a chat", async () => {
      await toolCallHandler({
        requestId: "req-7",
        toolCall: { name: "get_messages", arguments: { chat_id: "1" } },
      });

      await vi.waitFor(() => {
        expect(mockTransportEmit).toHaveBeenCalled();
      });

      const result = getEmittedResult("toolResult");
      expect(result).toBeDefined();
      expect(result!.isError).not.toBe(true);
      expect(result!.content).toBeDefined();
    });
  });

  describe("Error handling", () => {
    it("should return error for unknown tool", async () => {
      await toolCallHandler({
        requestId: "req-8",
        toolCall: { name: "unknown_tool", arguments: {} },
      });

      await vi.waitFor(() => {
        expect(mockTransportEmit).toHaveBeenCalled();
      });

      const result = getEmittedResult("toolResult");
      expect(result).toBeDefined();
      expect(result!.isError).toBe(true);

      const errorText = JSON.stringify(result!.content).toLowerCase();
      expect(errorText).toContain("unknown_tool");
      expect(errorText).toMatch(/not found/);
    });

    it("should handle rate limit errors", async () => {
      // Mock enforceRateLimit to reject for this call
      const rateLimiter = await import("../../mcp/rateLimiter");
      vi.mocked(rateLimiter.enforceRateLimit).mockRejectedValueOnce(
        new Error("Rate limit exceeded")
      );

      await toolCallHandler({
        requestId: "req-9",
        toolCall: {
          name: "send_message",
          arguments: { chat_id: "1", message: "Test message" },
        },
      });

      await vi.waitFor(() => {
        expect(mockTransportEmit).toHaveBeenCalled();
      });

      const result = getEmittedResult("toolResult");
      expect(result).toBeDefined();
      expect(result!.isError).toBe(true);

      const errorText = JSON.stringify(result!.content).toLowerCase();
      expect(errorText).toMatch(/rate limit/);
    });

    it("should handle API errors gracefully", async () => {
      // Make the send_message tool handler throw
      const sendMessageApi = await import("../api/sendMessage");
      vi.mocked(sendMessageApi.sendMessage).mockRejectedValueOnce(
        new Error("Network error: Connection timeout")
      );

      await toolCallHandler({
        requestId: "req-10",
        toolCall: {
          name: "send_message",
          arguments: { chat_id: "1", message: "Test" },
        },
      });

      await vi.waitFor(() => {
        expect(mockTransportEmit).toHaveBeenCalled();
      });

      const result = getEmittedResult("toolResult");
      expect(result).toBeDefined();
      expect(result!.isError).toBe(true);
    });
  });

  describe("Transport integration", () => {
    it("should register event handlers on construction", () => {
      expect(mockTransportOn).toHaveBeenCalledWith(
        "toolCall",
        expect.any(Function)
      );
      expect(mockTransportOn).toHaveBeenCalledWith(
        "listTools",
        expect.any(Function)
      );
    });

    it("should emit toolResult with correct requestId", async () => {
      const requestId = "unique-req-id-123";

      await toolCallHandler({
        requestId,
        toolCall: { name: "get_chats", arguments: {} },
      });

      await vi.waitFor(() => {
        expect(mockTransportEmit).toHaveBeenCalledWith(
          "toolResult",
          expect.objectContaining({ requestId })
        );
      });
    });

    it("should emit listToolsResponse with correct requestId", () => {
      const requestId = "list-unique-123";

      listToolsHandler({ requestId });

      expect(mockTransportEmit).toHaveBeenCalledWith(
        "listToolsResponse",
        expect.objectContaining({ requestId })
      );
    });
  });
});
