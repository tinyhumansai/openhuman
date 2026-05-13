import crypto from "node:crypto";

export const DEFAULT_PORT = 18473;
export const MOCK_JWT = "e2e-mock-jwt-token";
export const MAX_PORT_RETRY_ATTEMPTS = 10;

let requestLog = [];
let mockBehavior = {};
let mockTunnels = [];

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
  return Number.isFinite(value) && value > 0 ? value : 0;
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
  return {
    _id: "user-123",
    telegramId: 12345678,
    hasAccess: true,
    magicWord: "alpha",
    firstName: "Test",
    lastName: "User",
    username: "testuser",
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
  return {
    team: {
      _id: "team-1",
      name: "Personal",
      slug: "personal",
      createdBy: "test-user-123",
      isPersonal: true,
      maxMembers: 1,
      subscription: {
        plan,
        hasActiveSubscription: isActive,
        planExpiry: expiry,
      },
      usage: {
        dailyTokenLimit: 1000,
        remainingTokens: 1000,
        activeSessionCount: 0,
      },
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString(),
    },
    role: "ADMIN",
  };
}
