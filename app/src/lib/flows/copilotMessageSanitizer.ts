import createDebug from 'debug';

/**
 * copilotMessageSanitizer — defends the Flows copilot chat against rendering
 * the raw provider wire-format envelope instead of clean assistant text
 * (B25).
 *
 * The Rust core's `NativeToolDispatcher::to_provider_messages`
 * (`src/openhuman/agent/dispatcher.rs`) and the tinyagents bridge's
 * `message_to_native_chat_message` (`src/openhuman/tinyagents/convert.rs`)
 * serialize an assistant turn that both talks AND calls a tool into a
 * `{ "content": "...", "tool_calls": [...] }` JSON envelope — the provider
 * wire format used to round-trip tool-calling history on the NEXT request.
 * That envelope is meant to stay internal to the agent session; this
 * sanitizer is a shape-based (not string-match) belt-and-suspenders guard so
 * that if it ever leaks into a chat message's `content` (from any Rust-side
 * vector, present or future), the copilot still renders only the human
 * text — never the raw JSON — mirroring the `unwrapPayloadEnvelope`
 * philosophy from PR #4822 (`app/src/lib/flows/runItems.ts`).
 */
const log = createDebug('app:flows:copilot-sanitizer');

/** Top-level keys the native tool-call envelope may carry. */
const ENVELOPE_KEYS = new Set(['content', 'tool_calls', 'reasoning_content']);

/** A single entry of the envelope's `tool_calls` array. */
interface EnvelopeToolCall {
  name?: unknown;
}

/** Result of unwrapping a chat message's raw `content` string. */
export interface UnwrappedToolCallMessage {
  /** The human-readable text to render (never the raw JSON). */
  text: string;
  /** Tool names found on the envelope's `tool_calls` array, in order. */
  toolNames: string[];
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/**
 * If `raw` is a serialized `{ "content": "...", "tool_calls": [...] }` JSON
 * envelope (the provider wire format for a native tool-calling turn), extract
 * and return only the human-readable `content` text plus the tool names found
 * (for chip rendering). Otherwise returns `raw` unchanged.
 *
 * Shape-based, not string-match: only collapses when the parsed value is a
 * plain object whose keys are ALL drawn from the known envelope key set AND a
 * string `content` key is present — so ordinary JSON-looking assistant prose
 * (e.g. a code block containing `{"content": "hi"}`) is never misread as an
 * envelope unless it structurally matches. Never silently drops data: any
 * non-envelope shape (including malformed JSON, or JSON with unexpected
 * top-level keys) passes `raw` straight through unchanged.
 */
export function unwrapToolCallEnvelope(raw: string): UnwrappedToolCallMessage {
  if (typeof raw !== 'string' || raw.trim().length === 0) {
    return { text: raw ?? '', toolNames: [] };
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    log('unwrapToolCallEnvelope: not JSON — pass through (chars=%d)', raw.length);
    return { text: raw, toolNames: [] };
  }

  if (!isPlainObject(parsed)) {
    log('unwrapToolCallEnvelope: JSON but not an object — pass through');
    return { text: raw, toolNames: [] };
  }

  const keys = Object.keys(parsed);
  if (keys.length === 0 || !keys.every(key => ENVELOPE_KEYS.has(key))) {
    log('unwrapToolCallEnvelope: non-envelope object (keys=%o) — pass through', keys);
    return { text: raw, toolNames: [] };
  }

  // Require `tool_calls` to be an array before treating this as a tool-call
  // envelope at all — a content-only object like `{ "content": "hi" }` is
  // ordinary JSON-looking prose, not a native envelope, and must pass
  // through unchanged rather than being unwrapped down to `hi`.
  if (!Array.isArray(parsed.tool_calls)) {
    log(
      'unwrapToolCallEnvelope: envelope-shaped keys=%o but `tool_calls` is not an array — pass through',
      keys
    );
    return { text: raw, toolNames: [] };
  }
  const toolCalls = parsed.tool_calls as EnvelopeToolCall[];

  // The native dispatcher can serialize a tool-only turn with `content: null`
  // (or omit `content` entirely) while still including `tool_calls`. Treat
  // null/missing `content` as `''` so the bubble still exposes the tool
  // activity chip instead of falling back to the raw JSON.
  const { content } = parsed;
  if (content !== null && content !== undefined && typeof content !== 'string') {
    log(
      'unwrapToolCallEnvelope: envelope-shaped keys=%o but `content` is non-string (and not null/missing) — pass through',
      keys
    );
    return { text: raw, toolNames: [] };
  }
  const text = typeof content === 'string' ? content : '';

  const toolNames = toolCalls
    .map(call => call?.name)
    .filter((name): name is string => typeof name === 'string' && name.length > 0);

  log(
    'unwrapToolCallEnvelope: envelope keys=%o — extracted text (chars=%d) + toolNames=%o',
    keys,
    text.length,
    toolNames
  );
  return { text, toolNames };
}
