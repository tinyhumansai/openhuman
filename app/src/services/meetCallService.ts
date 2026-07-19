// Frontend service for the "Join a Google Meet call" feature.
//
// Two-phase request:
//  1. Call the core RPC `openhuman.meet_join_call` to validate inputs and
//     mint a stable `request_id`. The core also logs the request — useful
//     for an eventual call audit trail.
//  2. Invoke the Tauri command `meet_call_open_window` to actually open
//     the dedicated CEF webview window at the Meet URL.
//
// Splitting it this way keeps platform-specific window code in the shell
// while the validation rules live (and are tested) in the core.
import { invoke } from '@tauri-apps/api/core';
import debug from 'debug';

import { isTauri } from '../utils/tauriCommands/common';
import { apiClient } from './apiClient';
import { callCoreRpc } from './coreRpcClient';

// Shares the sibling hook's namespace (`useMeetingMascots.ts`) so the whole
// dual-mascot flow can be traced under one prefix.
const log = debug('meet:mascots');

/**
 * Map optional Rive colors to the backend's snake_case wire shape, trimming
 * blanks and collapsing an all-empty pair to `undefined`. Shared by the
 * top-level and per-slot color payloads so the two can't drift.
 */
function mapRiveColors(colors?: {
  primaryColor?: string;
  secondaryColor?: string;
}): { primary_color?: string; secondary_color?: string } | undefined {
  if (!colors) return undefined;
  const primary = colors.primaryColor?.trim() || undefined;
  const secondary = colors.secondaryColor?.trim() || undefined;
  if (!primary && !secondary) return undefined;
  return { primary_color: primary, secondary_color: secondary };
}

export type MeetJoinCallInput = {
  meetUrl: string;
  /** Bot's display name in Meet's "Your name" prompt. */
  displayName: string;
  /**
   * The launching user's display name as it will appear in the Meet
   * call. This is the *only* speaker the in-call wake-word gate will
   * accept — captions from any other participant are dropped before
   * tools can be dispatched. Empty / missing fails closed in core
   * (no wakes fire) which is the safe default during the rollout.
   */
  ownerDisplayName?: string;
  /**
   * ElevenLabs voice id for the primary mascot (issue #4277). When two
   * mascots are enabled the core alternates the speaking voice per reply.
   * Omit for single-mascot calls (core keeps its default voice).
   */
  primaryVoiceId?: string;
  /** Voice id for the secondary mascot; present only in two-mascot calls. */
  secondaryVoiceId?: string;
};

export type MeetJoinCallResult = {
  requestId: string;
  meetUrl: string;
  displayName: string;
  ownerDisplayName: string;
  windowLabel: string;
};

type CoreJoinResponse = { ok: boolean; request_id: string; meet_url: string; display_name: string };

export async function joinMeetCall(input: MeetJoinCallInput): Promise<MeetJoinCallResult> {
  const meetUrl = input.meetUrl.trim();
  const displayName = input.displayName.trim();
  const ownerDisplayName = (input.ownerDisplayName ?? '').trim();

  if (!meetUrl) throw new Error('Please paste a Google Meet link.');
  if (!displayName) throw new Error('Please enter a display name.');
  // Owner name is the privacy lock — captions from anyone else are
  // refused by the core wake gate. Surfacing the requirement up front
  // keeps the user from sitting through the join only to find the bot
  // ignores them; matches the message the inline alert would show.
  if (!ownerDisplayName) {
    throw new Error(
      'Please enter your own name as it will appear in the Meet so OpenHuman knows who to listen to.'
    );
  }
  // Refuse early outside the desktop shell so the browser dev surface
  // (`pnpm dev`) doesn't mint a stray request_id on the core for a join
  // attempt that has no chance of opening a CEF window.
  if (!isTauri()) {
    throw new Error(
      'Joining a Meet call requires the desktop app. Run `pnpm tauri dev` and try again.'
    );
  }

  const rpcResult = await callCoreRpc<CoreJoinResponse>({
    method: 'openhuman.meet_join_call',
    params: { meet_url: meetUrl, display_name: displayName },
  });

  if (!rpcResult?.ok || !rpcResult.request_id) {
    throw new Error('Core rejected the meet_join_call request.');
  }

  let windowLabel: string;
  try {
    windowLabel = await invoke<string>('meet_call_open_window', {
      args: {
        request_id: rpcResult.request_id,
        meet_url: rpcResult.meet_url,
        display_name: rpcResult.display_name,
        // Owner name doesn't round-trip through meet_join_call (the
        // RPC is platform-agnostic validation only) — pass it
        // directly to the shell so the meet_audio start path can
        // hand it to the wake-word gate. See feat/mascot-meet-flowA
        // Plan C — owner-only privacy lock.
        owner_display_name: ownerDisplayName,
        // Per-mascot voices for speaker alternation (issue #4277). Absent
        // → core keeps its single default voice (unchanged behavior).
        primary_voice_id: input.primaryVoiceId?.trim() || undefined,
        secondary_voice_id: input.secondaryVoiceId?.trim() || undefined,
      },
    });
  } catch (err) {
    // Tauri v2 rejects with a String (the Err side of `Result<_, String>`),
    // not a JS Error. Wrap so the UI catch block — which checks
    // `instanceof Error` — surfaces the real reason instead of a fallback.
    const reason =
      err instanceof Error ? err.message : typeof err === 'string' ? err : JSON.stringify(err);
    console.error('[meet-call] meet_call_open_window invoke rejected:', err);
    throw new Error(`meet_call_open_window failed: ${reason}`);
  }

  return {
    requestId: rpcResult.request_id,
    meetUrl: rpcResult.meet_url,
    displayName: rpcResult.display_name,
    ownerDisplayName,
    windowLabel,
  };
}

export async function closeMeetCall(requestId: string): Promise<boolean> {
  if (!isTauri()) return false;
  return invoke<boolean>('meet_call_close_window', { requestId });
}

/**
 * One completed Meet call as persisted by the core in the JSONL
 * recent-calls log (written by `handle_stop_session`). Same shape
 * as `MeetCallRecord` in `src/openhuman/meet_agent/store.rs` —
 * snake_case fields because the core surfaces them verbatim.
 */
export interface MeetCallRecord {
  request_id: string;
  meet_url: string;
  bot_display_name: string;
  owner_display_name: string;
  started_at_ms: number;
  ended_at_ms: number;
  listened_seconds: number;
  spoken_seconds: number;
  turn_count: number;
  /**
   * Distinct human participant display names mined from the transcript
   * (backend-meet flow). Older records and local meet-agent calls omit this,
   * so it is optional and defaults to an empty list at the UI.
   */
  participants?: string[];
}

/** One transcript line of a recorded call. Mirrors `MeetCallTranscriptLine`. */
export interface MeetCallTranscriptLine {
  /** Lowercased speaker role: `'participant'` or `'assistant'`. */
  role: string;
  /** The line as the backend delivered it (may carry a `[MM:SS] [Name]` prefix). */
  content: string;
}

/** One action item mined from a call. Mirrors `MeetCallActionItem`. */
export interface MeetCallActionItem {
  description: string;
  /** `'executable'` or `'advisory'`. */
  kind: string;
  tool_name?: string | null;
  assignee?: string | null;
}

/** Structured post-call summary. Mirrors `MeetCallSummary`. */
export interface MeetCallSummary {
  headline: string;
  key_points: string[];
  action_items: MeetCallActionItem[];
}

/**
 * Transcript + summary for one completed call. Mirrors `MeetCallDetail` in
 * `src/openhuman/meet_agent/store.rs`. Lazy-loaded by the recent-calls panel
 * when a row is expanded, so the list payload stays lean. `summary` is null
 * when summarisation failed or timed out at call-end.
 */
export interface MeetCallDetail {
  request_id: string;
  summary?: MeetCallSummary | null;
  transcript: MeetCallTranscriptLine[];
}

interface CoreListCallsResponse {
  ok: boolean;
  calls: MeetCallRecord[];
  count: number;
}

interface CoreGetCallDetailResponse {
  ok: boolean;
  detail: MeetCallDetail | null;
}

/**
 * Fetch the most recent completed Meet calls (newest first). Used
 * by the Skills "Meeting Bots" modal to render a history list
 * underneath the join form. Returns an empty array on a fresh
 * install (no recorded calls yet) — the core treats a missing
 * JSONL file as "no rows" rather than an error.
 */
export async function listMeetCalls(limit = 20): Promise<MeetCallRecord[]> {
  const result = await callCoreRpc<CoreListCallsResponse>({
    method: 'openhuman.meet_agent_list_calls',
    params: { limit },
  });
  if (!result?.ok) {
    throw new Error('Core rejected the meet_agent_list_calls request.');
  }
  return result.calls ?? [];
}

/**
 * Fetch the transcript + summary for one completed call. Lazy-loaded when the
 * user expands a recent-call row. Returns `null` when the core has no detail
 * for this call (older calls recorded before the feature, or a failed write) —
 * the panel renders a "no transcript yet" state in that case.
 */
export async function getMeetCallDetail(requestId: string): Promise<MeetCallDetail | null> {
  const result = await callCoreRpc<CoreGetCallDetailResponse>({
    method: 'openhuman.meet_agent_get_call_detail',
    params: { request_id: requestId },
  });
  if (!result?.ok) {
    throw new Error('Core rejected the meet_agent_get_call_detail request.');
  }
  return result.detail ?? null;
}

// ---------------------------------------------------------------------------
// Transcript parsing
// ---------------------------------------------------------------------------

/** A transcript line with its parsed timestamp/speaker prefix stripped out. */
export interface ParsedTranscriptLine {
  timestamp: string | null;
  speaker: string | null;
  text: string;
  role: string;
}

const TRANSCRIPT_PREFIX_RE = /^\[(\d{1,2}:\d{2})\]\s*\[([^\]]+)\]\s*(.*)/s;

/**
 * Parse a raw transcript line's content for the optional `[MM:SS] [Name]` prefix.
 * When the prefix is present, returns the parsed timestamp, speaker, and remaining text.
 * When absent, timestamp and speaker are null and text is the full content.
 */
export function parseTranscriptLine(line: MeetCallTranscriptLine): ParsedTranscriptLine {
  const match = TRANSCRIPT_PREFIX_RE.exec(line.content);
  if (match) {
    return {
      timestamp: match[1] ?? null,
      speaker: match[2] ?? null,
      text: match[3] ?? '',
      role: line.role,
    };
  }
  return { timestamp: null, speaker: null, text: line.content, role: line.role };
}

// ---------------------------------------------------------------------------
// Backend Meet Bot via Core Socket.IO bridge
// ---------------------------------------------------------------------------

export type MeetingPlatform = 'gmeet' | 'zoom' | 'teams' | 'webex';

export type BackendMeetJoinInput = {
  meetUrl: string;
  displayName?: string;
  platform?: MeetingPlatform;
  agentName?: string;
  systemPrompt?: string;
  mascotId?: string;
  riveColors?: { primaryColor?: string; secondaryColor?: string };
  /**
   * Dual-mascot config (issue #4277). Up to 2 slots; slot 0 = primary,
   * slot 1 = secondary. When two are present the backend bot renders both
   * mascots and alternates who speaks each reply (each with its own
   * `voiceId`). Omit / single element = legacy single-mascot behavior via
   * `mascotId`.
   */
  mascots?: Array<{
    mascotId: string;
    /**
     * Human-facing mascot name (from the manifest, e.g. "Toshi"). Enables
     * name-addressed routing (#4277 follow-up): a participant who says
     * "Hey Toshi …" is answered by this slot instead of the mechanical
     * alternation. Omit → that slot is not name-addressable.
     */
    name?: string;
    riveColors?: { primaryColor?: string; secondaryColor?: string };
    voiceId?: string;
  }>;
  /** Only respond to messages from this participant name (empty = respond to all). */
  respondToParticipant?: string;
  /** Wake phrase the participant must say before the bot responds (empty = no wake phrase). */
  wakePhrase?: string;
  /** Opaque correlation id echoed on all `bot:*` events for this session. */
  correlationId?: string;
  /** When true, the bot joins in listen-only mode (no microphone, no replies). */
  listenOnly?: boolean;
};

type CoreBackendMeetJoinResponse = { ok: boolean; meet_url: string; platform: string };

/**
 * Join a meeting via the backend's Recall.ai bot. Supports Google Meet,
 * Zoom, Microsoft Teams, and Webex.
 *
 * Calls the core RPC `openhuman.agent_meetings_join`, which emits `bot:join`
 * over the core's persistent Socket.IO connection to the backend. The backend
 * streams events back (`bot:reply`, `bot:harness`, `bot:transcript`, `bot:left`)
 * which the core bridges to the frontend as `agent_meetings:*` socket events.
 */
export async function joinMeetViaBackendBot(
  input: BackendMeetJoinInput
): Promise<{ meetUrl: string; platform: string }> {
  const meetUrl = input.meetUrl.trim();
  if (!meetUrl) throw new Error('Please paste a meeting link.');

  // Dual-mascot slots (issue #4277), mapped to the backend's snake_case wire
  // shape. Absent → backend falls back to `mascot_id`.
  const slots = input.mascots?.filter(m => m.mascotId?.trim());
  const mascots =
    slots && slots.length > 0
      ? slots.map(m => ({
          mascot_id: m.mascotId.trim(),
          name: m.name?.trim() || undefined,
          voice_id: m.voiceId?.trim() || undefined,
          rive_colors: mapRiveColors(m.riveColors),
        }))
      : undefined;

  // Flow/state metadata only — no participant names, voices, or the meet URL.
  log(
    'backend bot join corr=%s dual=%s slots=%d singleMascot=%s riveColors=%s',
    input.correlationId?.trim() || '-',
    Boolean(input.mascots?.length),
    mascots?.length ?? 0,
    Boolean(input.mascotId?.trim()),
    Boolean(mapRiveColors(input.riveColors))
  );

  const result = await callCoreRpc<CoreBackendMeetJoinResponse>({
    method: 'openhuman.agent_meetings_join',
    params: {
      meet_url: meetUrl,
      display_name: input.displayName?.trim() || undefined,
      platform: input.platform || undefined,
      agent_name: input.agentName?.trim() || undefined,
      system_prompt: input.systemPrompt?.trim() || undefined,
      mascot_id: input.mascotId?.trim() || undefined,
      respond_to_participant: input.respondToParticipant?.trim() || undefined,
      wake_phrase: input.wakePhrase?.trim() || undefined,
      correlation_id: input.correlationId?.trim() || undefined,
      listen_only: input.listenOnly ?? undefined,
      rive_colors: mapRiveColors(input.riveColors),
      mascots,
    },
  });

  if (!result?.ok) {
    throw new Error('Core rejected the agent_meetings_join request.');
  }

  return { meetUrl: result.meet_url, platform: result.platform };
}

/**
 * Ask the backend bot to leave the current meeting.
 */
export async function leaveBackendMeetBot(reason?: string): Promise<void> {
  await callCoreRpc<{ ok: boolean }>({
    method: 'openhuman.agent_meetings_leave',
    params: { reason: reason || 'requested' },
  });
}

/**
 * Send a tool execution result back to the backend's meeting LLM.
 */
export async function sendHarnessResponse(result: string): Promise<void> {
  await callCoreRpc<{ ok: boolean }>({
    method: 'openhuman.agent_meetings_harness_response',
    params: { result },
  });
}

/**
 * Direct backend-driven meet bot join.
 *
 * Hits `POST /mascots/join-meeting` which:
 *  - gates free users with a 429 (SERVER_OVERLOADED) — surfaced verbatim
 *    so callers can show the user-facing capacity message;
 *  - launches the Recall.ai mascot bot for supported meeting platforms.
 *
 * The app normally uses `joinMeetViaBackendBot`, which routes through the
 * core Socket.IO bridge so backend bot events can be handled locally too.
 */
/** Alias of {@link MeetingPlatform} — kept for existing consumers. */
export type MascotMeetPlatform = MeetingPlatform;

export interface MascotJoinMeetingInput {
  platform: MascotMeetPlatform;
  meetUrl: string;
  displayName?: string;
}

export interface MascotJoinMeetingResult {
  success: boolean;
  data?: unknown;
}

/**
 * Tailored, actionable user-facing copy shown when the backend's capacity gate
 * trips — replaces the backend's terse "…Please try again later." with retry
 * guidance, without leaking the underlying paid-plan rule.
 */
export const SERVER_OVERLOADED_MESSAGE =
  'OpenHuman is under heavy load right now. Please try again in a few minutes.';

/**
 * Recognize the backend's free-user capacity-gate response (`SERVER_OVERLOADED`,
 * backend `paidPlan.ts` → `"Mascot streaming capacity is exhausted. Please try
 * again later."`).
 *
 * The shared `apiClient` drops `errorCode` from error bodies (`apiClient.ts`
 * only forwards `error` + `message`), so the message text is the only signal
 * that survives. Detection therefore MUST key on the backend wording — it used
 * to be compared for exact equality against [`SERVER_OVERLOADED_MESSAGE`], but
 * that constant was changed to friendlier copy and no longer matches the
 * backend string, so the check silently never fired and the raw generic
 * "…try again later." leaked to the user instead of the tailored notice
 * (#4151). Match a stable substring, case-insensitively, so minor wording drift
 * on either side still resolves to the actionable message.
 */
export function isCapacityGateMessage(text: string | null | undefined): boolean {
  if (!text) return false;
  const t = text.toLowerCase();
  return t.includes('streaming capacity') || t.includes('capacity is exhausted');
}

export interface MascotJoinMeetingError {
  /** User-safe error text. Falls back to a generic message. */
  message: string;
  /** True when the backend returned the 429 capacity gate. */
  isCapacityGated: boolean;
}

function isApiErrorLike(value: unknown): value is { error?: unknown; message?: unknown } {
  return !!value && typeof value === 'object' && ('error' in value || 'message' in value);
}

// ---------------------------------------------------------------------------
// Upcoming meetings (meet_list_upcoming RPC)
// ---------------------------------------------------------------------------

/**
 * One upcoming calendar meeting returned by `openhuman.meet_list_upcoming`.
 * Mirrors `UpcomingMeeting` in `src/openhuman/agent_meetings/types.rs`.
 */
export interface UpcomingMeeting {
  calendar_event_id: string;
  title: string;
  /** Unix milliseconds */
  start_time_ms: number;
  /** Unix milliseconds */
  end_time_ms: number;
  meet_url: string | null;
  /** Platform slug: "gmeet" | "zoom" | "teams" | "webex" */
  platform: string | null;
  participant_count: number | null;
  organizer: string | null;
  /** "auto" | "ask" | "skip" — local UI state only this phase */
  join_policy: string;
  calendar_source: string;
}

interface CoreListUpcomingResponse {
  ok: boolean;
  meetings: UpcomingMeeting[];
}

/**
 * Fetch upcoming calendar meetings that have a conferencing link.
 * Returns an empty array when no Google Calendar is connected.
 */
export async function listUpcomingMeetings(
  lookaheadMinutes?: number,
  limit?: number
): Promise<UpcomingMeeting[]> {
  const result = await callCoreRpc<CoreListUpcomingResponse>({
    method: 'openhuman.meet_list_upcoming',
    params: {
      ...(lookaheadMinutes != null ? { lookahead_minutes: lookaheadMinutes } : {}),
      ...(limit != null ? { limit } : {}),
    },
  });
  if (!result?.ok) {
    throw new Error('Core rejected the meet_list_upcoming request.');
  }
  return result.meetings ?? [];
}

// ---------------------------------------------------------------------------
// Per-event join-policy overrides
// ---------------------------------------------------------------------------

interface CoreSetEventPolicyResponse {
  ok: boolean;
}

interface CoreGetEventPoliciesResponse {
  ok: boolean;
  policies: Record<string, string>;
}

/**
 * Persist a per-event join-policy override for a specific calendar event.
 * Resolution order (Rust side): per-event > per-platform > global.
 */
export async function setEventPolicy(
  calendarEventId: string,
  policy: 'auto' | 'ask' | 'skip'
): Promise<void> {
  const result = await callCoreRpc<CoreSetEventPolicyResponse>({
    method: 'openhuman.meet_set_event_policy',
    params: { calendar_event_id: calendarEventId, policy },
  });
  if (!result?.ok) {
    throw new Error('Core rejected the meet_set_event_policy request.');
  }
}

/**
 * Batch-fetch per-event join-policy overrides for the given calendar event IDs.
 * Only IDs that have an explicit override are present in the returned map.
 */
export async function getEventPolicies(
  calendarEventIds: string[]
): Promise<Record<string, string>> {
  if (calendarEventIds.length === 0) return {};
  const result = await callCoreRpc<CoreGetEventPoliciesResponse>({
    method: 'openhuman.meet_get_event_policies',
    params: { calendar_event_ids: calendarEventIds },
  });
  if (!result?.ok) {
    throw new Error('Core rejected the meet_get_event_policies request.');
  }
  return result.policies ?? {};
}

export async function joinMeetingViaMascotBot(
  input: MascotJoinMeetingInput
): Promise<MascotJoinMeetingResult> {
  const meetUrl = input.meetUrl.trim();
  if (!meetUrl) {
    throw { message: 'Please paste a meeting link.', isCapacityGated: false };
  }
  try {
    return await apiClient.post<MascotJoinMeetingResult>('/mascots/join-meeting', {
      platform: input.platform,
      meetUrl,
      displayName: input.displayName?.trim() || undefined,
    });
  } catch (err) {
    // apiClient throws `{ success:false, error, message? }`. The 429 body
    // is `{ error: SERVER_OVERLOADED_MESSAGE, errorCode: 'SERVER_OVERLOADED' }`
    // — `errorCode` is dropped by the shared client (see apiClient.ts:96),
    // so we detect capacity by matching the canonical message.
    const text = isApiErrorLike(err)
      ? typeof err.error === 'string'
        ? err.error
        : typeof err.message === 'string'
          ? err.message
          : 'Failed to start meeting bot.'
      : err instanceof Error
        ? err.message
        : 'Failed to start meeting bot.';
    const isCapacityGated = isCapacityGateMessage(text);
    // When capacity-gated, surface the tailored, actionable copy instead of the
    // backend's raw "…try again later." string (#4151).
    const wrapped: MascotJoinMeetingError = {
      message: isCapacityGated ? SERVER_OVERLOADED_MESSAGE : text,
      isCapacityGated,
    };
    throw wrapped;
  }
}
