/**
 * Fold a v2 orchestration transcript into renderable rows.
 *
 * The harness stream delivers a `tool_call` and its `tool_result` as two
 * separate rows correlated by `callId`. For rendering we want them as ONE unit
 * (command + output + outcome), so `mergeToolActivity` pairs them and derives a
 * single `failed` flag from the result's `isError` / `exitCode` / `ok`. Every
 * other row passes through unchanged. Pure — unit-tested in isolation.
 */
import type { ChatMessage } from './useOrchestrationChats';

/** A merged tool_call (+ its tool_result when present). */
export interface ToolActivity {
  kind: 'tool';
  /** Stable key: the tool_call id (or the orphan result id). */
  id: string;
  toolName?: string;
  /** The tool_call body (command / display). Empty for an orphan result. */
  command: string;
  /** The tool_result body (output), once it has arrived. */
  output?: string;
  callId?: string;
  /** True once the matching tool_result exists. */
  hasResult: boolean;
  /** The tool run failed — `isError`, a non-zero `exitCode`, or `ok === false`. */
  failed: boolean;
  timestamp: string;
}

/** A plain (non-tool) transcript row, rendered as a bubble/inline block. */
export interface MessageRow {
  kind: 'message';
  message: ChatMessage;
}

export type TranscriptRow = ToolActivity | MessageRow;

/** Whether a `tool_result` row represents a failure. */
export function toolResultFailed(
  result: Pick<ChatMessage, 'ok' | 'isError' | 'exitCode'>
): boolean {
  return result.isError === true || (result.exitCode ?? 0) > 0 || result.ok === false;
}

function activityFromCall(call: ChatMessage): ToolActivity {
  return {
    kind: 'tool',
    id: call.id,
    ...(call.toolName ? { toolName: call.toolName } : {}),
    command: call.body,
    ...(call.callId ? { callId: call.callId } : {}),
    hasResult: false,
    failed: false,
    timestamp: call.timestamp,
  };
}

function applyResult(activity: ToolActivity, result: ChatMessage): ToolActivity {
  return {
    ...activity,
    output: result.body,
    hasResult: true,
    failed: toolResultFailed(result),
    ...(activity.toolName ? {} : result.toolName ? { toolName: result.toolName } : {}),
  };
}

/**
 * Merge `tool_call`/`tool_result` pairs (by `callId`) into single `tool` rows.
 * A result with no matching prior call renders as its own tool row (command
 * empty). Order is preserved by the position of the `tool_call` (or an orphan
 * result). Non-tool rows are wrapped as `message` rows.
 */
export function mergeToolActivity(messages: ChatMessage[]): TranscriptRow[] {
  const rows: TranscriptRow[] = [];
  // callId → index into `rows` of the open tool activity awaiting its result.
  const openByCallId = new Map<string, number>();

  for (const message of messages) {
    if (message.eventKind === 'tool_call') {
      const activity = activityFromCall(message);
      if (message.callId) openByCallId.set(message.callId, rows.length);
      rows.push(activity);
      continue;
    }
    if (message.eventKind === 'tool_result') {
      const openIndex = message.callId ? openByCallId.get(message.callId) : undefined;
      if (openIndex !== undefined) {
        const open = rows[openIndex] as ToolActivity;
        rows[openIndex] = applyResult(open, message);
        if (message.callId) openByCallId.delete(message.callId);
      } else {
        // Orphan result (call not seen) — render standalone.
        const orphan = applyResult(activityFromCall({ ...message, body: '' }), message);
        rows.push(orphan);
      }
      continue;
    }
    rows.push({ kind: 'message', message });
  }

  return rows;
}
