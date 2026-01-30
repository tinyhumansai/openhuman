/**
 * MCP Rate Limiter
 *
 * Three-tier tool classification:
 *   1. STATE_ONLY  — reads cached Redux state, zero API calls → no rate limit
 *   2. API_READ    — reads from Telegram API → standard inter-call delay
 *   3. API_WRITE   — mutates state on Telegram servers → heavy inter-call delay
 *
 * On top of the per-call delay, two budget caps apply to all API-bound tools:
 *   - Per-request counter (caps tool calls within a single agent request)
 *   - Per-minute sliding window (prevents sustained high-frequency usage)
 */

import { mcpLog, mcpWarn } from "./logger";

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

export const RATE_LIMIT_CONFIG = {
  /** Minimum delay (ms) between API read calls */
  API_READ_DELAY_MS: 500,
  /** Delay (ms) between API write / mutation calls */
  API_WRITE_DELAY_MS: 1000,
  /** Maximum API-bound tool calls within a 60-second sliding window */
  MAX_CALLS_PER_MINUTE: 30,
  /** Maximum API-bound tool calls within a single MCP request */
  MAX_CALLS_PER_REQUEST: 20,
} as const;

// ---------------------------------------------------------------------------
// Tool classification
// ---------------------------------------------------------------------------

export type ToolTier = "state_only" | "api_read" | "api_write";

/**
 * Tools that ONLY read from cached Redux state — zero Telegram API calls.
 * Bypass all rate limiting; execute instantly.
 */
const STATE_ONLY_TOOLS = new Set<string>([
  // Chat state (selectOrderedChats / state.chats)
  "get_chats",
  "list_chats",
  "get_chat",

  // Message state (state.messages / state.messagesOrder)
  "get_messages",
  "list_messages",
  "get_message_context",
  "get_history",

  // Current user (state.currentUser)
  "get_me",

  // Derived from cached chat/message data
  "list_inline_buttons",
  "get_contact_chats",
  "get_direct_chat_by_contact",

  // These read from cached messages only (no API call)
  "get_last_interaction",
  "get_media_info",
]);

/**
 * Tools that call the Telegram API but only READ data (no mutations).
 * Subject to standard inter-call delay + per-minute/per-request caps.
 */
const API_READ_TOOLS = new Set<string>([
  // Contacts / users (contacts.GetContacts, contacts.Search, etc.)
  "list_contacts",
  "search_contacts",
  "get_contact_ids",
  "get_blocked_users",
  "get_user_status",
  "get_user_photos",

  // Chat metadata (channels.GetParticipants, messages.GetFullChat, etc.)
  "get_participants",
  "get_admins",
  "get_banned_users",
  "get_recent_actions",
  "get_bot_info",
  "get_privacy_settings",

  // Messages (messages.Search, messages.GetMessagesReactions, etc.)
  "get_pinned_messages",
  "get_message_reactions",
  "search_messages",

  // Drafts / misc reads
  "get_drafts",
  "get_sticker_sets",
  "get_gif_search",

  // Topics (channels.GetForumTopics)
  "list_topics",

  // Discovery (these call the Telegram API for server-side search)
  "search_public_chats",
  "resolve_username",
  "export_contacts",
]);

/**
 * Tools that MODIFY state on Telegram servers.
 * Subject to heavy inter-call delay + per-minute/per-request caps.
 */
const API_WRITE_TOOLS = new Set<string>([
  // Message mutations
  "send_message",
  "reply_to_message",
  "edit_message",
  "delete_message",
  "forward_message",
  "pin_message",
  "unpin_message",
  "mark_as_read",
  "send_reaction",
  "remove_reaction",
  "save_draft",
  "clear_draft",
  "press_inline_button",
  "create_poll",

  // Invite link (generates/exports a link — treated as write)
  "get_invite_link",

  // Chat mutations
  "create_group",
  "create_channel",
  "invite_to_group",
  "edit_chat_title",
  "edit_chat_photo",
  "delete_chat_photo",
  "leave_chat",
  "archive_chat",
  "unarchive_chat",
  "mute_chat",
  "unmute_chat",
  "export_chat_invite",
  "import_chat_invite",
  "join_chat_by_link",
  "subscribe_public_channel",

  // Admin / moderation
  "promote_admin",
  "demote_admin",
  "ban_user",
  "unban_user",

  // Contact mutations
  "add_contact",
  "delete_contact",
  "block_user",
  "unblock_user",
  "import_contacts",

  // Profile mutations
  "update_profile",
  "set_profile_photo",
  "delete_profile_photo",
  "set_privacy_settings",
  "set_bot_commands",
]);

// ---------------------------------------------------------------------------
// Rate limiter state
// ---------------------------------------------------------------------------

/** Timestamp of the last API-bound tool call */
let lastCallTime = 0;

/** Per-request call counter — reset via resetRequestCallCount() */
let callsInCurrentRequest = 0;

/** Sliding window of timestamps for per-minute tracking */
const callHistory: number[] = [];

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/**
 * Classify a tool into one of the three tiers.
 * Unknown tools default to api_read (safe fallback — rate limited but not heavy).
 */
export function classifyTool(toolName: string): ToolTier {
  if (STATE_ONLY_TOOLS.has(toolName)) return "state_only";
  if (API_WRITE_TOOLS.has(toolName)) return "api_write";
  if (API_READ_TOOLS.has(toolName)) return "api_read";
  // Unknown tools default to api_read so they're rate limited
  return "api_read";
}

/**
 * Returns true if the tool only reads from local cache (no API call).
 */
export function isStateOnlyTool(toolName: string): boolean {
  return STATE_ONLY_TOOLS.has(toolName);
}

/**
 * Returns true if the tool performs a mutation/write via the Telegram API.
 */
export function isHeavyTool(toolName: string): boolean {
  return API_WRITE_TOOLS.has(toolName);
}

/** @deprecated Use isStateOnlyTool instead */
export const isReadOnlyTool = isStateOnlyTool;

/**
 * Reset the per-request call counter. Call at the start of each new
 * MCP request (agent turn) to allow a fresh budget of tool calls.
 */
export function resetRequestCallCount(): void {
  callsInCurrentRequest = 0;
}

/**
 * Enforce rate limits before executing a tool.
 *
 * - State-only tools skip all limits (instant).
 * - API-bound tools (read or write):
 *   1. Check per-request budget → throw if exceeded
 *   2. Check per-minute sliding window → sleep until budget available
 *   3. Enforce inter-call delay (500ms for reads, 1000ms for writes)
 *
 * Call BEFORE executing the tool handler. May sleep or throw.
 */
export async function enforceRateLimit(toolName: string): Promise<void> {
  const tier = classifyTool(toolName);

  // State-only tools are always allowed instantly
  if (tier === "state_only") {
    return;
  }

  // --- Per-request cap ---
  callsInCurrentRequest += 1;
  if (callsInCurrentRequest > RATE_LIMIT_CONFIG.MAX_CALLS_PER_REQUEST) {
    throw new Error(
      `Rate limit: exceeded ${RATE_LIMIT_CONFIG.MAX_CALLS_PER_REQUEST} API tool calls per request. ` +
        `Try breaking your task into smaller steps.`,
    );
  }

  // --- Per-minute sliding window ---
  const now = Date.now();
  purgeOldEntries(now);

  if (callHistory.length >= RATE_LIMIT_CONFIG.MAX_CALLS_PER_MINUTE) {
    const oldestTimestamp = callHistory[0];
    const waitMs = oldestTimestamp + 60_000 - now + 50; // +50ms buffer
    mcpWarn(
      `Rate limit: per-minute cap reached (${RATE_LIMIT_CONFIG.MAX_CALLS_PER_MINUTE}/min). ` +
        `Waiting ${waitMs}ms for '${toolName}'.`,
    );
    await sleep(waitMs);
    purgeOldEntries(Date.now());
  }

  // --- Inter-call delay (tier-dependent) ---
  const requiredDelay =
    tier === "api_write"
      ? RATE_LIMIT_CONFIG.API_WRITE_DELAY_MS
      : RATE_LIMIT_CONFIG.API_READ_DELAY_MS;

  const elapsed = Date.now() - lastCallTime;
  if (elapsed < requiredDelay) {
    const waitMs = requiredDelay - elapsed;
    mcpLog(
      `Rate limit: ${tier} delay ${waitMs}ms for '${toolName}'`,
    );
    await sleep(waitMs);
  }

  // Record this call
  lastCallTime = Date.now();
  callHistory.push(lastCallTime);
}

/**
 * Get current rate limit status for diagnostics / debugging.
 */
export function getRateLimitStatus(): {
  callsThisRequest: number;
  callsThisMinute: number;
  lastCallAgoMs: number;
} {
  purgeOldEntries(Date.now());
  return {
    callsThisRequest: callsInCurrentRequest,
    callsThisMinute: callHistory.length,
    lastCallAgoMs: lastCallTime > 0 ? Date.now() - lastCallTime : -1,
  };
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

function purgeOldEntries(now: number): void {
  const cutoff = now - 60_000;
  while (callHistory.length > 0 && callHistory[0] < cutoff) {
    callHistory.shift();
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, Math.max(0, ms)));
}
