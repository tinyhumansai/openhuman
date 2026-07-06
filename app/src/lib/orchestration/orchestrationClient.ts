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

/** External agent harness that emits a session (drives the roster grouping). */
export type HarnessType = 'claude' | 'codex' | 'gemini';

/**
 * Coarse instance status for the roster dot. Peer instances carry no true
 * run-state yet, so the core derives only `idle` / `stopped` today; the
 * remaining states are modelled here (and by `InstanceStatusDot`) for the
 * attention-queue and run-state follow-ups.
 */
export type InstanceStatus = 'running' | 'idle' | 'waiting-approval' | 'errored' | 'stopped';

export interface SessionSummary {
  sessionId: string;
  agentId: string;
  source: string;
  /** Emitting harness when this is an external instance; absent for master/subconscious/user-created. */
  harnessType?: HarnessType;
  /** Coarse status for the roster dot (see {@link InstanceStatus}). */
  status: InstanceStatus;
  /** One-line current activity (latest message preview) for the roster. */
  currentTask?: string;
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

export interface SessionCreateResponse {
  session: SessionSummary;
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

/** One @handle this agent's wallet holds (reverse-resolved from the directory). */
export interface SelfHandle {
  username: string;
  primary: boolean;
}

/**
 * This agent's own tiny.place identity and whether peers can reach it.
 *
 * `discoverable` is the bottom line: a peer can DM this agent only when both its
 * directory card (`cardPublished`) and its Signal encryption key (`keyPublished`)
 * are live. A fresh identity can accept contacts yet stay un-messageable until it
 * registers a @handle — the card surfaces that gap instead of a mystery 404.
 */
export interface SelfIdentity {
  agentId: string;
  handles: SelfHandle[];
  primaryHandle?: string;
  cardPublished: boolean;
  keyPublished: boolean;
  discoverable: boolean;
}

/** The relay endpoint the core talks to, plus a coarse network label. */
export interface RelayInfo {
  baseUrl: string;
  network: 'staging' | 'prod';
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

  /** Create a new empty session for a contact; returns the created summary. */
  sessionsCreate: (params: { agentId: string; label?: string }) =>
    call<SessionCreateResponse>('openhuman.orchestration_sessions_create', {
      agentId: params.agentId,
      ...(params.label !== undefined ? { label: params.label } : {}),
    }),

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

  /**
   * Send a message from the human master. With `sessionId` the message threads
   * under that session (session envelope); otherwise it goes to the Master chat.
   */
  sendMasterMessage: (params: { body: string; recipient?: string; sessionId?: string }) =>
    call<SendMasterMessageResponse>('openhuman.orchestration_send_master_message', {
      body: params.body,
      ...(params.recipient !== undefined ? { recipient: params.recipient } : {}),
      ...(params.sessionId !== undefined ? { sessionId: params.sessionId } : {}),
    }),

  /** Mark a chat as read (clears the server-side unread count). */
  markRead: (chat: string) => call<MarkReadResponse>('openhuman.orchestration_mark_read', { chat }),

  /** Current orchestration status (active steering directive, tick timing). */
  status: () => call<OrchestrationStatus>('openhuman.orchestration_status', {}),

  /**
   * This agent's own tiny.place identity + discoverability (agent id, @handles,
   * whether its directory card and Signal key are published, whether peers can
   * DM it). Powers the SelfIdentityCard.
   */
  selfIdentity: () => call<SelfIdentity>('openhuman.orchestration_self_identity', {}),

  /** The relay endpoint + network label the core is talking to (RelayBadge). */
  relayInfo: () => call<RelayInfo>('openhuman.orchestration_relay_info', {}),
};

export type OrchestrationClient = typeof orchestrationClient;
