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

import { isTauri } from '../utils/tauriCommands/common';
import { apiClient } from './apiClient';
import { callCoreRpc } from './coreRpcClient';

export type MeetJoinCallInput = { meetUrl: string; displayName: string };

export type MeetJoinCallResult = {
  requestId: string;
  meetUrl: string;
  displayName: string;
  windowLabel: string;
};

type CoreJoinResponse = { ok: boolean; request_id: string; meet_url: string; display_name: string };

export async function joinMeetCall(input: MeetJoinCallInput): Promise<MeetJoinCallResult> {
  const meetUrl = input.meetUrl.trim();
  const displayName = input.displayName.trim();

  if (!meetUrl) throw new Error('Please paste a Google Meet link.');
  if (!displayName) throw new Error('Please enter a display name.');
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
    windowLabel,
  };
}

export async function closeMeetCall(requestId: string): Promise<boolean> {
  if (!isTauri()) return false;
  return invoke<boolean>('meet_call_close_window', { requestId });
}

/**
 * Backend-driven meet bot join (PR tinyhumansai/backend#773).
 *
 * Hits `POST /mascots/join-meeting` which:
 *  - gates free users with a 429 (SERVER_OVERLOADED) — surfaced verbatim
 *    so callers can show the user-facing capacity message;
 *  - launches the Camoufox mascot bot for `gmeet`;
 *  - 400s on `zoom` / `teams` with "not yet supported".
 *
 * Distinct from `joinMeetCall` (which opens a CEF webview locally) —
 * this is a fire-and-forget request that runs the mascot bot in the
 * backend and streams events over Socket.IO.
 */
export type MascotMeetPlatform = 'gmeet' | 'zoom' | 'teams';

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
 * The 429 capacity-gate message the backend emits for free users. Treated
 * as the canonical user-facing copy so the UI can show a tailored notice
 * without leaking the underlying paid-plan rule.
 */
export const SERVER_OVERLOADED_MESSAGE =
  'OpenHuman is under heavy load right now. Please try again in a few minutes.';

export interface MascotJoinMeetingError {
  /** User-safe error text. Falls back to a generic message. */
  message: string;
  /** True when the backend returned the 429 capacity gate. */
  isCapacityGated: boolean;
}

function isApiErrorLike(value: unknown): value is { error?: unknown; message?: unknown } {
  return !!value && typeof value === 'object' && ('error' in value || 'message' in value);
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
    const isCapacityGated = text === SERVER_OVERLOADED_MESSAGE;
    const wrapped: MascotJoinMeetingError = { message: text, isCapacityGated };
    throw wrapped;
  }
}
