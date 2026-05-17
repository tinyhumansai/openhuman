import crypto from "node:crypto";

export const DEFAULT_PORT = 18473;
export const MOCK_JWT = "e2e-mock-jwt-token";
export const MAX_PORT_RETRY_ATTEMPTS = 10;
export const MAX_MOCK_DELAY_MS = 30_000;

let requestLog = [];
let mockBehavior = {};
let mockTunnels = [];
let mockConversations = [];
let mockMessages = [];
let mockCronJobs = [];
let mockWebhookTriggers = [];
let socketEventLog = [];
let mockLlmThreads = new Map();
let nextSequence = 1;

const socketSessions = new Map();

export const openSockets = new Set();

export function getRequestLog() {
  return [...requestLog];
}

export function clearRequestLog() {
  requestLog = [];
}

export function appendRequest(entry) {
  requestLog.push(entry);
}

export function nextMockSequence() {
  const value = nextSequence;
  nextSequence += 1;
  return value;
}

export function getMockBehavior() {
  return { ...mockBehavior };
}

export function setMockBehavior(key, value) {
  mockBehavior[key] = String(value);
}

export function setMockBehaviors(behavior, mode = "merge") {
  if (mode === "replace") {
    mockBehavior = {};
  }
  for (const [key, value] of Object.entries(behavior || {})) {
    mockBehavior[key] = String(value);
  }
}

export function resetMockBehavior() {
  mockBehavior = {};
}

export function behavior() {
  return mockBehavior;
}

export function parseInteger(value, fallback = 0) {
  const parsed = Number(value);
  return Number.isInteger(parsed) ? parsed : fallback;
}

export function hashString(input) {
  const text = String(input ?? "");
  let hash = 2166136261;
  for (let i = 0; i < text.length; i += 1) {
    hash ^= text.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  return hash >>> 0;
}

export function behaviorSeed() {
  return String(mockBehavior.seed || "openhuman-mock");
}

export function fuzzyNumber(label, min, max) {
  const lower = Math.min(min, max);
  const upper = Math.max(min, max);
  if (lower === upper) return lower;
  const hash = hashString(`${behaviorSeed()}:${label}`);
  return lower + (hash % (upper - lower + 1));
}

export function fuzzyPick(label, values, fallback = null) {
  if (!Array.isArray(values) || values.length === 0) return fallback;
  const index = fuzzyNumber(label, 0, values.length - 1);
  return values[index];
}

export function fuzzyTimestamp(label, spreadMs = 14 * 24 * 60 * 60 * 1000) {
  const now = Date.now();
  const offset = fuzzyNumber(label, 0, spreadMs);
  return new Date(now - offset).toISOString();
}

export function createMockId(prefix = "mock") {
  return `${prefix}_${nextMockSequence()}_${hashString(
    `${prefix}:${behaviorSeed()}:${Date.now()}:${Math.random()}`,
  )
    .toString(16)
    .slice(0, 8)}`;
}

export function parseBehaviorJson(key, fallback) {
  const raw = mockBehavior[key];
  if (!raw) return JSON.parse(JSON.stringify(fallback));
  try {
    return JSON.parse(raw);
  } catch {
    return JSON.parse(JSON.stringify(fallback));
  }
}

export function getDelayMs(key) {
  const value = Number(mockBehavior[key] || 0);
  if (!Number.isFinite(value) || value <= 0) return 0;
  return Math.min(value, MAX_MOCK_DELAY_MS);
}

export function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export function getMockTunnels() {
  return mockTunnels;
}

export function setMockTunnels(next) {
  mockTunnels = Array.isArray(next) ? next : [];
}

export function resetMockTunnels() {
  mockTunnels = [];
}

export function getMockConversations() {
  return mockConversations;
}

export function setMockConversations(next) {
  mockConversations = Array.isArray(next) ? next : [];
}

export function resetMockConversations() {
  mockConversations = [];
}

export function getMockMessages() {
  return mockMessages;
}

export function setMockMessages(next) {
  mockMessages = Array.isArray(next) ? next : [];
}

export function resetMockMessages() {
  mockMessages = [];
}

export function getMockCronJobs() {
  return mockCronJobs;
}

export function setMockCronJobs(next) {
  mockCronJobs = Array.isArray(next) ? next : [];
}

export function resetMockCronJobs() {
  mockCronJobs = [];
}

export function getMockWebhookTriggers() {
  return mockWebhookTriggers;
}

export function setMockWebhookTriggers(next) {
  mockWebhookTriggers = Array.isArray(next) ? next : [];
}

export function resetMockWebhookTriggers() {
  mockWebhookTriggers = [];
}

export function listMockLlmThreads() {
  return Array.from(mockLlmThreads.values()).map((thread) => ({
    key: thread.key,
    createdAt: thread.createdAt,
    updatedAt: thread.updatedAt,
    turnCount: thread.turnCount || 0,
    lastModel: thread.lastModel || null,
    lastFamily: thread.lastFamily || null,
    lastUserMessage: thread.lastUserMessage || null,
    lastAssistantContent: thread.lastAssistantContent || null,
    lastToolCallCount: Array.isArray(thread.lastToolCalls)
      ? thread.lastToolCalls.length
      : 0,
  }));
}

export function getMockLlmThread(key) {
  return mockLlmThreads.get(String(key)) || null;
}

export function touchMockLlmThread(key, patch = {}) {
  if (!key) return null;
  const threadKey = String(key);
  const current = getMockLlmThread(threadKey) || {
    key: threadKey,
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
    turnCount: 0,
    history: [],
    lastModel: null,
    lastFamily: null,
    lastUserMessage: null,
    lastAssistantContent: null,
    lastToolCalls: [],
    lastToolResult: null,
    lastCodeLanguage: null,
  };
  const next = {
    ...current,
    ...patch,
    updatedAt: new Date().toISOString(),
  };
  mockLlmThreads.set(threadKey, next);
  return next;
}

export function recordMockLlmTurn(key, turn = {}) {
  if (!key) return null;
  const thread = touchMockLlmThread(key);
  const history = Array.isArray(thread.history) ? [...thread.history] : [];
  history.push({
    timestamp: new Date().toISOString(),
    requestText: turn.requestText || null,
    responseText: turn.responseText || null,
    toolCalls: Array.isArray(turn.toolCalls) ? turn.toolCalls : [],
    toolResultText: turn.toolResultText || null,
    model: turn.model || null,
    family: turn.family || null,
  });
  const next = touchMockLlmThread(key, {
    history: history.slice(-12),
    turnCount: (thread.turnCount || 0) + 1,
    lastModel: turn.model || thread.lastModel || null,
    lastFamily: turn.family || thread.lastFamily || null,
    lastUserMessage: turn.requestText || thread.lastUserMessage || null,
    lastAssistantContent:
      turn.responseText ?? thread.lastAssistantContent ?? null,
    lastToolCalls: Array.isArray(turn.toolCalls)
      ? turn.toolCalls
      : (thread.lastToolCalls ?? []),
    lastToolResult: turn.toolResultText ?? thread.lastToolResult ?? null,
    lastCodeLanguage: turn.codeLanguage ?? thread.lastCodeLanguage ?? null,
  });
  return next;
}

export function resetMockLlmThreads() {
  mockLlmThreads = new Map();
}

export function listSocketSessions() {
  return Array.from(socketSessions.values()).map((session) => ({
    sid: session.sid,
    socketId: session.socketId,
    connected: session.connected === true,
    upgradedToWebSocket: session.upgradedToWebSocket === true,
    transport: session.transport || "polling",
    userId: session.userId || null,
    tokenPresent: Boolean(session.token),
    createdAt: session.createdAt,
    lastSeenAt: session.lastSeenAt,
    outgoingQueueLength: Array.isArray(session.pendingPackets)
      ? session.pendingPackets.length
      : 0,
  }));
}

export function getSocketSession(sid) {
  return socketSessions.get(String(sid)) || null;
}

export function registerSocketSession(session) {
  const next = {
    sid: session.sid,
    socketId: session.socketId || session.sid,
    connected: false,
    upgradedToWebSocket: false,
    transport: session.transport || "polling",
    token: session.token || null,
    userId: session.userId || null,
    createdAt: session.createdAt || new Date().toISOString(),
    lastSeenAt: new Date().toISOString(),
    pendingPackets: [],
    webSocket: null,
  };
  socketSessions.set(next.sid, next);
  return next;
}

export function touchSocketSession(sid, patch = {}) {
  const session = getSocketSession(sid);
  if (!session) return null;
  Object.assign(session, patch, { lastSeenAt: new Date().toISOString() });
  return session;
}

export function attachWebSocketToSession(sid, webSocket, options = {}) {
  return touchSocketSession(sid, {
    webSocket,
    ...(options.transport ? { transport: options.transport } : {}),
    ...(typeof options.upgraded === "boolean"
      ? { upgradedToWebSocket: options.upgraded }
      : {}),
  });
}

export function dropSocketSession(sid) {
  const session = getSocketSession(sid);
  if (!session) return;
  try {
    session.webSocket?.destroy?.();
  } catch {
    // noop
  }
  socketSessions.delete(session.sid);
}

export function queueSocketPacket(sid, packet) {
  const session = getSocketSession(sid);
  if (!session) return false;
  session.pendingPackets.push(String(packet));
  session.lastSeenAt = new Date().toISOString();
  return true;
}

export function drainSocketPackets(sid) {
  const session = getSocketSession(sid);
  if (!session) return [];
  const packets = [...session.pendingPackets];
  session.pendingPackets = [];
  session.lastSeenAt = new Date().toISOString();
  return packets;
}

export function appendSocketEvent(entry) {
  socketEventLog.push({
    id: createMockId("sockevt"),
    timestamp: new Date().toISOString(),
    ...entry,
  });
}

export function getSocketEventLog() {
  return [...socketEventLog];
}

export function clearSocketEventLog() {
  socketEventLog = [];
}

export function resetSocketSessions() {
  for (const sid of socketSessions.keys()) {
    dropSocketSession(sid);
  }
  socketSessions.clear();
  clearSocketEventLog();
}

export function buildSocketReadyPayload(session) {
  return {
    sid: session.socketId,
    userId: session.userId || "mock-user",
    transport: session.transport,
  };
}

export function createMockTunnel(payload = {}) {
  const now = new Date().toISOString();
  return {
    id: crypto.randomUUID(),
    uuid: crypto.randomUUID(),
    name: String(payload.name || "Mock Tunnel").trim(),
    description: String(payload.description || "").trim(),
    isActive: payload.isActive ?? true,
    createdAt: now,
    updatedAt: now,
  };
}

export function getMockUser() {
  const firstName =
    mockBehavior.firstName ||
    fuzzyPick(
      "user:first",
      ["Test", "Casey", "Robin", "Jordan", "Taylor"],
      "Test",
    );
  const lastName =
    mockBehavior.lastName ||
    fuzzyPick(
      "user:last",
      ["User", "Walker", "Lane", "Rivera", "Stone"],
      "User",
    );
  const username =
    mockBehavior.username ||
    `${String(firstName).toLowerCase()}${fuzzyNumber("user:username", 10, 99)}`;
  return {
    _id: mockBehavior.userId || "user-123",
    telegramId: fuzzyNumber("user:telegramId", 10_000_000, 99_999_999),
    hasAccess: true,
    magicWord:
      mockBehavior.magicWord ||
      fuzzyPick(
        "user:magicWord",
        ["alpha", "delta", "spruce", "harbor", "ember"],
        "alpha",
      ),
    firstName,
    lastName,
    username,
    role: "user",
    activeTeamId: "team-1",
    referral: {},
    subscription: { hasActiveSubscription: false, plan: "FREE" },
    settings: {
      dailySummariesEnabled: false,
      dailySummaryChatIds: [],
      autoCompleteEnabled: false,
      autoCompleteVisibility: "always",
      autoCompleteWhitelistChatIds: [],
      autoCompleteBlacklistChatIds: [],
    },
    usage: {
      cycleBudgetUsd: 10,
      remainingUsd: 10,
      spentThisCycleUsd: 0,
      spentTodayUsd: 0,
      cycleStartDate: new Date().toISOString(),
    },
    autoDeleteTelegramMessagesAfterDays: 30,
    autoDeleteThreadsAfterDays: 30,
  };
}

export function getMockTeam() {
  const plan = mockBehavior.plan || "FREE";
  const isActive = mockBehavior.planActive === "true";
  const expiry = mockBehavior.planExpiry || null;
  const teamName =
    mockBehavior.teamName ||
    fuzzyPick(
      "team:name",
      ["Personal", "Studio", "Field Ops", "Research"],
      "Personal",
    );
  return {
    team: {
      _id: "team-1",
      name: teamName,
      slug: String(teamName).toLowerCase().replace(/\s+/g, "-"),
      createdBy: "test-user-123",
      isPersonal: true,
      maxMembers: 1,
      subscription: {
        plan,
        hasActiveSubscription: isActive,
        planExpiry: expiry,
      },
      usage: {
        dailyTokenLimit: fuzzyNumber("team:dailyTokenLimit", 500, 5000),
        remainingTokens: fuzzyNumber("team:remainingTokens", 100, 5000),
        activeSessionCount: listSocketSessions().length,
      },
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString(),
    },
    role: "ADMIN",
  };
}
