/**
 * Meet Agent service — Stage 1.
 *
 * Thin typed wrapper around the two Tauri commands that spawn / tear down
 * the hidden Google Meet agent webview, plus a subscription helper for the
 * lifecycle events the agent script emits.
 *
 * Commands (Tauri → Rust):
 *   webview_meet_agent_join   → spawns a hidden webview, auto-joins the meeting
 *   webview_meet_agent_leave  → politely leaves and closes the webview
 *
 * Events received (via the existing `webview:event` Tauri channel):
 *   meet_agent_joined  { accountId, code, joinedAt }
 *   meet_agent_left    { accountId, reason }
 *   meet_agent_failed  { accountId, reason }
 */
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import debug from 'debug';

import { isTauri } from './webviewAccountService';

const log = debug('meet-agent');
const errLog = debug('meet-agent:error');

// ─── Public types ──────────────────────────────────────────────────────────

export type MeetAgentEvent =
  | { kind: 'meet_agent_joined'; accountId: string; code: string; joinedAt: number }
  | { kind: 'meet_agent_left'; accountId: string; reason: string }
  | { kind: 'meet_agent_failed'; accountId: string; reason: string };

// ─── Commands ──────────────────────────────────────────────────────────────

/**
 * Spawn a hidden Google Meet agent webview and begin auto-joining.
 *
 * No-ops gracefully when not running inside Tauri (e.g. web dev mode).
 * The webview shares the cookie store of the existing google-meet webview
 * for the same account, so no re-login is required.
 */
export async function meetAgentJoin(args: {
  accountId: string;
  meetingUrl: string;
}): Promise<void> {
  if (!isTauri()) {
    log('[meet-agent] meetAgentJoin no-op outside Tauri accountId=%s', args.accountId);
    return;
  }
  log('[meet-agent] meetAgentJoin accountId=%s meetingUrl=%s', args.accountId, args.meetingUrl);
  try {
    await invoke('webview_meet_agent_join', { args });
    log('[meet-agent] meetAgentJoin invoked ok');
  } catch (err) {
    errLog('[meet-agent] meetAgentJoin failed: %o', err);
    throw err;
  }
}

/**
 * Gracefully leave the meeting and close the agent webview.
 *
 * Idempotent — safe to call even if the agent was never started or has
 * already been torn down.
 */
export async function meetAgentLeave(args: { accountId: string }): Promise<void> {
  if (!isTauri()) {
    log('[meet-agent] meetAgentLeave no-op outside Tauri accountId=%s', args.accountId);
    return;
  }
  log('[meet-agent] meetAgentLeave accountId=%s', args.accountId);
  try {
    await invoke('webview_meet_agent_leave', { args });
    log('[meet-agent] meetAgentLeave invoked ok');
  } catch (err) {
    errLog('[meet-agent] meetAgentLeave failed: %o', err);
    throw err;
  }
}

// ─── Event subscription ────────────────────────────────────────────────────

/**
 * The envelope the Rust side fires on `webview:event`.
 * Matches the payload shape from `webview_recipe_event` / runtime.js.
 */
interface WebviewEventEnvelope {
  account_id: string;
  provider: string;
  kind: string;
  payload: Record<string, unknown>;
  ts?: number | null;
}

const AGENT_EVENT_KINDS = new Set(['meet_agent_joined', 'meet_agent_left', 'meet_agent_failed']);

/**
 * Subscribe to lifecycle events emitted by the Meet agent webview.
 *
 * Returns an unsubscribe function. Safe to call outside Tauri (returns a
 * no-op unsubscribe).
 *
 * Filters out all non-agent events from the shared `webview:event` channel.
 */
export function subscribeMeetAgentEvents(handler: (e: MeetAgentEvent) => void): () => void {
  if (!isTauri()) {
    log('[meet-agent] subscribeMeetAgentEvents no-op outside Tauri');
    return () => {};
  }

  let cancelFn: (() => void) | null = null;
  let active = true;

  void listen<WebviewEventEnvelope>('webview:event', evt => {
    const { kind, payload, account_id } = evt.payload;
    if (!AGENT_EVENT_KINDS.has(kind)) return;
    log('[meet-agent] received event kind=%s accountId=%s', kind, account_id);

    try {
      if (kind === 'meet_agent_joined') {
        handler({
          kind: 'meet_agent_joined',
          accountId: account_id,
          code: String(payload.code ?? ''),
          joinedAt: Number(payload.joinedAt ?? Date.now()),
        });
      } else if (kind === 'meet_agent_left') {
        handler({
          kind: 'meet_agent_left',
          accountId: account_id,
          reason: String(payload.reason ?? 'unknown'),
        });
      } else if (kind === 'meet_agent_failed') {
        handler({
          kind: 'meet_agent_failed',
          accountId: account_id,
          reason: String(payload.reason ?? 'unknown'),
        });
      }
    } catch (err) {
      errLog('[meet-agent] handler threw: %o', err);
    }
  }).then(
    unlisten => {
      if (!active) {
        // Unsubscribe was called before the promise resolved.
        unlisten();
      } else {
        cancelFn = unlisten;
      }
    },
    err => {
      errLog('[meet-agent] listen() failed: %o', err);
    }
  );

  return () => {
    active = false;
    if (cancelFn) {
      cancelFn();
      cancelFn = null;
    }
    log('[meet-agent] unsubscribed from meet agent events');
  };
}
