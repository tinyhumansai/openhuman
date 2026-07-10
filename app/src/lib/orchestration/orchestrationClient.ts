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
  /** Total persisted messages in the session; `0` for pinned/new windows. */
  messageCount?: number;
  active: boolean;
  pinned: boolean;
  /**
   * Live peer reachability from the core presence map. `true` = confidently
   * online (heard from within the TTL); `false` = confidently offline
   * (heartbeat); `undefined` = unknown → fall back to `active`/status.
   */
  peerOnline?: boolean;
  /** ISO-8601 last time the core heard from this peer, if ever. */
  lastSeenAt?: string;
}

/**
 * Typed kind of a harness (v2) event, mirroring the SDK's
 * `tinyplace.harness.session.v2` stream. Absent on legacy v1 rows (which carry
 * only role + body). The core hides `status`/`lifecycle`/`unknown` from the
 * thread, so those should not normally reach the renderer.
 */
export type HarnessEventKind =
  | 'user_prompt'
  | 'agent_message'
  | 'agent_thinking'
  | 'tool_call'
  | 'tool_result'
  | 'approval_request'
  | 'status'
  | 'lifecycle'
  | 'error'
  | 'unknown';

export interface OrchestrationMessage {
  id: string;
  agentId: string;
  sessionId: string;
  chatKind: OrchestrationChatKind;
  role: string;
  body: string;
  timestamp: string;
  seq: number;
  /** Typed event kind for v2 harness streams; absent on v1 text rows. */
  eventKind?: HarnessEventKind;
  /** Raw harness tool name (e.g. "Bash") on tool_call / tool_result rows. */
  toolName?: string;
  /** Correlation id shared by a tool_call and its tool_result. */
  callId?: string;
  /** tool_result outcome: whether the tool call succeeded. Absent off tool_result. */
  ok?: boolean;
  /** tool_result: the harness flagged the result as an error. */
  isError?: boolean;
  /** tool_result: process exit code when the tool was a command. */
  exitCode?: number;
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
  /** Whether the hosted brain was reachable at the last health sync. `false`
   * drives the "cloud brain unreachable" offline notice. Absent = assume up. */
  cloudReachable?: boolean;
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

// ── Attention queue ─────────────────────────────────────────────────────────

/** The kind of "needs you" signal, in descending urgency. */
export type AttentionKind = 'approval' | 'needs-input' | 'unread';

/** What the renderer should do when the user acts on an attention item. */
export type AttentionAction =
  | { type: 'approval'; requestId: string }
  | { type: 'open-thread'; threadId: string }
  | { type: 'open-run'; runId: string }
  | { type: 'open-session'; sessionId: string };

/** One actionable row in the attention queue. */
export interface AttentionItem {
  /** Stable list key (`<kind>:<source-id>`). */
  id: string;
  kind: AttentionKind;
  /** The instance/source this concerns (request id / run id / session id). */
  instanceId: string;
  /** Short label (tool name / agent display name / session label). */
  title: string;
  /** One-line detail; absent for `unread` (use `count`). */
  summary?: string;
  /** Unread message count; present only for the `unread` kind. */
  count?: number;
  action: AttentionAction;
  /** RFC3339 creation/activity time, when known. */
  createdAt?: string;
}

/** Per-kind + total counts for badging the zone. */
export interface AttentionCounts {
  total: number;
  approvals: number;
  needsInput: number;
  unread: number;
}

export interface AttentionQueue {
  items: AttentionItem[];
  counts: AttentionCounts;
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
   * The aggregated "needs you" queue: pending tool approvals, agent runs
   * awaiting input, and instances with unread messages, priority-ordered.
   */
  attention: () => call<AttentionQueue>('openhuman.orchestration_attention', {}),

  /**
   * This agent's own tiny.place identity + discoverability (agent id, @handles,
   * whether its directory card and Signal key are published, whether peers can
   * DM it). Powers the SelfIdentityCard.
   */
  selfIdentity: () => call<SelfIdentity>('openhuman.orchestration_self_identity', {}),

  /**
   * Make this agent discoverable: publish (or refresh) its directory card + Signal
   * encryption key for the wallet's current identity, then return the updated
   * {@link SelfIdentity}. No @handle registration and no payment — it repairs the
   * common "has an identity but card/key aren't published" gap that makes every
   * inbound DM 404. Powers the SelfIdentityCard's "Make discoverable" action.
   */
  publishIdentity: () => call<SelfIdentity>('openhuman.orchestration_publish_identity', {}),

  /** The relay endpoint + network label the core is talking to (RelayBadge). */
  relayInfo: () => call<RelayInfo>('openhuman.orchestration_relay_info', {}),
};

export type OrchestrationClient = typeof orchestrationClient;
