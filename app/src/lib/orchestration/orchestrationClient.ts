/**
 * Renderer client for the subconscious-orchestration Brain surface.
 *
 * Thin typed wrappers over the core `openhuman.orchestration_*` JSON-RPC
 * methods, routed through `callCoreRpc` exactly like the tiny.place bridge in
 * `invokeApiClient.ts`. The Rust core owns all business logic — this file is
 * only the transport seam.
 *
 * Error conventions mirror `invokeApiClient`:
 * - 402 Payment Required surfaces as {@link PaymentRequiredError} (re-exported
 *   here so callers do not need to reach into the tiny.place bridge).
 * - All other transport / RPC failures propagate as plain `Error`.
 */
import { callCoreRpc } from '../../services/coreRpcClient';
import { PaymentRequiredError } from '../agentworld/invokeApiClient';

export { PaymentRequiredError };

// ── Domain types (must match the Rust RPC shapes; do not rename) ──────────────

export type OrchestrationChatKind = 'master' | 'subconscious' | 'session';

export interface SessionSummary {
  sessionId: string;
  agentId: string;
  source: string;
  label?: string;
  workspace?: string;
  chatKind: OrchestrationChatKind;
  lastMessageAt: string;
  unread: number;
  active: boolean;
  pinned: boolean;
}

export interface OrchestrationMessage {
  id: string;
  agentId: string;
  sessionId: string;
  chatKind: OrchestrationChatKind;
  role: string;
  body: string;
  timestamp: string;
  seq: number;
}

export interface OrchestrationSteering {
  text: string;
  createdAt: string;
  expiresAfterCycles: number;
}

export interface OrchestrationStatus {
  steering?: OrchestrationSteering;
  lastTickAt?: number;
  ingestLastMessageAt?: string;
}

export interface SessionsListResponse {
  sessions: SessionSummary[];
}

export interface MessagesListResponse {
  messages: OrchestrationMessage[];
}

export interface SendMasterMessageResponse {
  ok: true;
  messageId: string;
}

export interface MarkReadResponse {
  ok: true;
}

/** Live socket event payload emitted by the core on new orchestration messages. */
export interface OrchestrationMessageEvent {
  agentId: string;
  sessionId: string;
  chatKind: string;
}

// ── Internal helper ───────────────────────────────────────────────────────────

function safeParseJson(s: string): unknown {
  try {
    return JSON.parse(s) as unknown;
  } catch {
    return s;
  }
}

/**
 * Call a `openhuman.orchestration_*` method and return the typed result.
 *
 * The core serialises 402 errors as a plain string `"PAYMENT_REQUIRED:<json>"`;
 * we decode it into a {@link PaymentRequiredError} so callers can render the
 * paywall state, matching `invokeApiClient`. All other errors propagate as-is.
 */
async function call<T>(method: string, params?: Record<string, unknown>): Promise<T> {
  try {
    return await callCoreRpc<T>({ method, params: params ?? {} });
  } catch (err) {
    const msg = String(err);
    const prefix = 'PAYMENT_REQUIRED:';
    const idx = msg.indexOf(prefix);
    if (idx >= 0) {
      throw new PaymentRequiredError(safeParseJson(msg.slice(idx + prefix.length)));
    }
    throw err;
  }
}

// ── Public API ────────────────────────────────────────────────────────────────

export const orchestrationClient = {
  /** List all orchestration chats (pinned master + subconscious, plus sessions). */
  sessionsList: () => call<SessionsListResponse>('openhuman.orchestration_sessions_list', {}),

  /**
   * List messages for a chat. `chat` is `"master"`, `"subconscious"`, or a
   * session's `sessionId`.
   */
  messagesList: (params: { chat: string; limit?: number; before?: string }) =>
    call<MessagesListResponse>('openhuman.orchestration_messages_list', {
      chat: params.chat,
      ...(params.limit !== undefined ? { limit: params.limit } : {}),
      ...(params.before !== undefined ? { before: params.before } : {}),
    }),

  /** Send a message from the human master into the pinned master chat. */
  sendMasterMessage: (params: { body: string; recipient?: string }) =>
    call<SendMasterMessageResponse>('openhuman.orchestration_send_master_message', {
      body: params.body,
      ...(params.recipient !== undefined ? { recipient: params.recipient } : {}),
    }),

  /** Mark a chat as read (clears the server-side unread count). */
  markRead: (chat: string) => call<MarkReadResponse>('openhuman.orchestration_mark_read', { chat }),

  /** Current orchestration status (active steering directive, tick timing). */
  status: () => call<OrchestrationStatus>('openhuman.orchestration_status', {}),
};

export type OrchestrationClient = typeof orchestrationClient;
