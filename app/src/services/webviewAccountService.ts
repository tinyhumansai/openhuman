import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import debug from 'debug';

import { store } from '../store';
import {
  appendLog,
  appendMessages,
  setAccountStatus,
  setActiveAccount,
} from '../store/accountsSlice';
import { addNotification } from '../store/notificationsSlice';
import type { AccountProvider, IngestedMessage } from '../types/accounts';
import { threadApi } from './api/threadApi';
import { chatSend } from './chatService';
import { callCoreRpc } from './coreRpcClient';
import { ingestNotification } from './notificationService';

const MEET_ORCHESTRATOR_MODEL = 'reasoning-v1';

const log = debug('webview-accounts');
const errLog = debug('webview-accounts:error');

interface RecipeEventPayload {
  account_id: string;
  provider: string;
  kind: 'ingest' | 'log' | 'notify' | string;
  payload: Record<string, unknown>;
  ts?: number | null;
}

interface IngestMessage {
  id?: string;
  from?: string | null;
  to?: string | null;
  fromMe?: boolean;
  body?: string | null;
  type?: string | null;
  timestamp?: number | null; // seconds since epoch
  unread?: number;
}

interface IngestPayload {
  messages?: IngestMessage[];
  // Legacy DOM-scrape fields (kept for non-whatsapp providers).
  unread?: number;
  snapshotKey?: string;
  // WPP-backed WhatsApp payload fields.
  provider?: string;
  chatId?: string;
  chatName?: string | null;
  day?: string; // YYYY-MM-DD UTC
  isSeed?: boolean;
}

interface NotificationClickPayload {
  account_id: string;
  provider: string;
}

interface RecipeNotifyPayload {
  title?: string;
  body?: string;
  icon?: string | null;
  tag?: string | null;
  silent?: boolean;
  [key: string]: unknown;
}

let unlisten: UnlistenFn | null = null;
let unlistenNotifyClick: UnlistenFn | null = null;
let started = false;
let permissionChecked = false;

export function startWebviewAccountService(): void {
  if (started) return;
  if (!isTauri()) {
    log('not in Tauri — webview accounts unavailable');
    return;
  }
  started = true;

  void (async () => {
    try {
      unlisten = await listen<RecipeEventPayload>('webview:event', evt => {
        handleRecipeEvent(evt.payload);
      });
      log('event listener attached');
    } catch (err) {
      errLog('failed to attach listener', err);
    }
    try {
      // Dormant until the platform click hook (UNUserNotificationCenter /
      // notify-rust on_response) emits `notification:click` from Rust.
      unlistenNotifyClick = await listen<NotificationClickPayload>('notification:click', evt => {
        handleNotificationClick(evt.payload);
      });
      log('notification:click listener attached');
    } catch (err) {
      errLog('failed to attach notification:click listener', err);
    }
  })();
}

export function stopWebviewAccountService(): void {
  if (unlisten) {
    unlisten();
    unlisten = null;
  }
  if (unlistenNotifyClick) {
    unlistenNotifyClick();
    unlistenNotifyClick = null;
  }
  started = false;
}

function handleNotificationClick(payload: NotificationClickPayload) {
  const accountId = payload?.account_id;
  const provider = payload?.provider;
  if (!accountId) {
    errLog('notification:click missing account_id — ignoring: %o', payload);
    return;
  }
  log('notification:click → account=%s provider=%s', accountId, provider);
  store.dispatch(setActiveAccount(accountId));
  void setFocusedAccount(accountId);
  invoke('activate_main_window').catch(err => {
    errLog('activate_main_window failed after notification click: %o', err);
  });
}

// Round-trip the OS notification permission once per session on first
// account open. Desktop plugin auto-grants today, but the shape matches
// the web API so future platform prompts slot in without UI change.
async function ensureNotificationPermission(): Promise<void> {
  if (permissionChecked) return;
  permissionChecked = true;
  try {
    const state = await invoke<string>('webview_notification_permission_state');
    log('notification permission state=%s', state);
    if (state === 'granted') return;
    const next = await invoke<string>('webview_notification_permission_request');
    log('notification permission after request=%s', next);
  } catch (err) {
    errLog('notification permission check failed: %o', err);
  }
}

function handleRecipeEvent(evt: RecipeEventPayload) {
  const accountId = evt.account_id;
  if (!accountId) return;

  if (evt.kind === 'log') {
    const level = (evt.payload.level as 'info' | 'warn' | 'error' | 'debug') || 'info';
    const msg = String(evt.payload.msg ?? '');
    store.dispatch(appendLog({ accountId, entry: { ts: evt.ts ?? Date.now(), level, msg } }));
    return;
  }

  // Google Meet lifecycle: the recipe emits these three event kinds to
  // drive the live-captions → transcript pipeline. Everything is
  // accumulated in-memory here; persistence fires once on meet_call_ended.
  if (evt.kind === 'meet_call_started') {
    handleMeetCallStarted(accountId, evt.payload as unknown as MeetCallStartedPayload);
    return;
  }
  if (evt.kind === 'meet_captions') {
    handleMeetCaptions(accountId, evt.payload as unknown as MeetCaptionsPayload);
    return;
  }
  if (evt.kind === 'meet_call_ended') {
    void handleMeetCallEnded(accountId, evt.payload as unknown as MeetCallEndedPayload);
    return;
  }

  if (evt.kind === 'ingest') {
    const ingest = evt.payload as IngestPayload;
    const messages: IngestedMessage[] = (ingest.messages ?? []).map((m, idx) => ({
      id: m.id ?? `${accountId}:${idx}`,
      from: m.from ?? null,
      body: m.body ?? null,
      unread: m.unread,
      ts: evt.ts ?? Date.now(),
    }));

    store.dispatch(appendMessages({ accountId, messages, unread: ingest.unread }));

    // Fire-and-forget memory write via the existing core RPC.
    // Namespace mirrors the skill-sync convention so the recall pipeline
    // can find these alongside other ingested context.
    void persistIngestToMemory(accountId, evt.provider, ingest, messages);
    return;
  }

  if (evt.kind === 'notify') {
    const payload = evt.payload as RecipeNotifyPayload;
    const title = String(payload.title ?? '').trim();
    const body = String(payload.body ?? '').trim();
    if (!title && !body) return;
    void ingestNotification({
      provider: evt.provider,
      account_id: accountId,
      title: title || `${evt.provider} notification`,
      body,
      raw_payload: payload as Record<string, unknown>,
    })
      .then(({ id }) => {
        store.dispatch(
          addNotification({
            id,
            provider: evt.provider,
            account_id: accountId,
            title: title || `${evt.provider} notification`,
            body,
            raw_payload: payload as Record<string, unknown>,
            status: 'unread',
            received_at: new Date().toISOString(),
          })
        );
      })
      .catch(err => {
        errLog('notify ingest failed account=%s provider=%s: %o', accountId, evt.provider, err);
      });
    return;
  }

  log('unhandled recipe event kind=%s account=%s', evt.kind, accountId);
}

async function persistIngestToMemory(
  accountId: string,
  provider: string,
  ingest: IngestPayload,
  messages: IngestedMessage[]
): Promise<void> {
  if (messages.length === 0) return;

  // WhatsApp (wa-js backed) sends one ingest event per (chatId, day) — a
  // stable key so repeated flushes of the same day upsert a single memory
  // doc. All other providers still use the legacy snapshot pattern.
  if (provider === 'whatsapp' && ingest.chatId && ingest.day) {
    await persistWhatsappChatDay(accountId, ingest);
    return;
  }

  const namespace = `webview:${provider}:${accountId}`;
  const key = ingest.snapshotKey
    ? `${namespace}:${hashKey(ingest.snapshotKey)}`
    : `${namespace}:${Date.now()}`;
  const title = `${provider} webview ingest — ${accountId.slice(0, 8)}`;
  const content = JSON.stringify(
    {
      provider,
      accountId,
      scrapedAt: new Date().toISOString(),
      unread: ingest.unread ?? 0,
      messages,
    },
    null,
    2
  );

  try {
    await callCoreRpc({
      method: 'openhuman.memory_doc_ingest',
      params: {
        namespace,
        key,
        title,
        content,
        source_type: 'webview-account',
        priority: 'low',
        tags: ['webview', provider],
        metadata: { provider, account_id: accountId },
        category: 'core',
      },
    });
    log('memory: ingested %d messages into %s', messages.length, namespace);
  } catch (err) {
    errLog('memory write failed for %s: %o', namespace, err);
  }
}

async function persistWhatsappChatDay(accountId: string, ingest: IngestPayload): Promise<void> {
  const chatId = ingest.chatId as string;
  const day = ingest.day as string;
  const chatName = ingest.chatName ?? chatId;
  const raw = ingest.messages ?? [];
  if (raw.length === 0) return;

  // Stable namespace + key: same (chat, day) always upserts the same doc.
  const namespace = `whatsapp-web:${accountId}`;
  const key = `${chatId}:${day}`;

  const sorted = [...raw].sort((a, b) => (a.timestamp ?? 0) - (b.timestamp ?? 0));

  const transcriptLines = sorted.map(m => {
    const tsSec = m.timestamp ?? 0;
    const hhmm = tsSec ? new Date(tsSec * 1000).toISOString().slice(11, 16) + 'Z' : '--:--';
    const who = m.fromMe ? 'me' : (m.from ?? '?');
    const body = (m.body ?? '').replace(/\r?\n/g, ' ').trim();
    const kind = m.type && m.type !== 'chat' ? ` [${m.type}]` : '';
    return `[${hhmm}] ${who}${kind}: ${body}`;
  });

  const header =
    `# WhatsApp — ${chatName} — ${day}\n` +
    `chat_id: ${chatId}\n` +
    `account_id: ${accountId}\n` +
    `messages: ${sorted.length}\n\n`;
  const content = header + transcriptLines.join('\n');

  const title = `WhatsApp · ${chatName} · ${day}`;

  try {
    await callCoreRpc({
      method: 'openhuman.memory_doc_ingest',
      params: {
        namespace,
        key,
        title,
        content,
        source_type: 'whatsapp-web',
        priority: 'medium',
        tags: ['whatsapp', 'chat-transcript', day],
        metadata: {
          provider: 'whatsapp',
          account_id: accountId,
          chat_id: chatId,
          chat_name: chatName,
          day,
          message_count: sorted.length,
          is_seed: !!ingest.isSeed,
        },
        category: 'core',
      },
    });
    log(
      'whatsapp: ingested %d msgs into %s key=%s (seed=%s)',
      sorted.length,
      namespace,
      key,
      !!ingest.isSeed
    );
  } catch (err) {
    errLog('whatsapp memory write failed %s key=%s: %o', namespace, key, err);
  }
}

function hashKey(input: string): string {
  // Simple non-cryptographic hash — just need a stable short key per snapshot.
  let h = 0;
  for (let i = 0; i < input.length; i += 1) {
    h = (h * 31 + input.charCodeAt(i)) | 0;
  }
  return Math.abs(h).toString(36);
}

// ────────────────────────────── Google Meet ─────────────────────────────
//
// Accumulate caption snapshots for each in-progress call and flush a
// single markdown transcript to memory when the meeting ends. Held
// purely in service-module memory — if the app is quit mid-call the
// buffer is lost, which matches the user expectation that Meet's
// built-in captions only live while the tab is open anyway.

interface MeetCaptionRow {
  speaker: string;
  text: string;
}

interface MeetCallStartedPayload {
  code: string;
  url?: string;
  startedAt: number;
}

interface MeetCaptionsPayload {
  code: string;
  captions: MeetCaptionRow[];
  ts: number;
}

interface MeetCallEndedPayload {
  code: string;
  endedAt: number;
  reason?: string;
}

interface CaptionSnapshot {
  ts: number;
  captions: MeetCaptionRow[];
}

interface MeetingSession {
  code: string;
  startedAt: number;
  snapshots: CaptionSnapshot[];
}

interface TranscriptSegment {
  speaker: string;
  text: string;
  startTs: number;
  endTs: number;
}

const MAX_MEET_SNAPSHOTS = 2000;

const activeMeetings = new Map<string, MeetingSession>();

function handleMeetCallStarted(accountId: string, payload: MeetCallStartedPayload) {
  // If there's a stale session (e.g. recipe missed the end for the
  // previous call), flush it first so we don't silently drop captions.
  const existing = activeMeetings.get(accountId);
  if (existing) {
    void flushMeetingSession(accountId, existing, Date.now(), 'superseded');
  }
  activeMeetings.set(accountId, {
    code: payload.code,
    startedAt: payload.startedAt,
    snapshots: [],
  });
  log('meet: call started account=%s code=%s', accountId, payload.code);
  store.dispatch(
    appendLog({
      accountId,
      entry: {
        ts: payload.startedAt,
        level: 'info',
        msg: `[meet] joined ${payload.code} — capturing captions`,
      },
    })
  );
}

function handleMeetCaptions(accountId: string, payload: MeetCaptionsPayload) {
  const session = activeMeetings.get(accountId);
  if (!session || session.code !== payload.code) return;
  session.snapshots.push({ ts: payload.ts, captions: payload.captions });
  if (session.snapshots.length > MAX_MEET_SNAPSHOTS) {
    // Long-tail buffer: drop the oldest. Worst case we lose the first
    // hour of a 4h meeting — the compression pass still works on tail.
    session.snapshots.splice(0, session.snapshots.length - MAX_MEET_SNAPSHOTS);
  }
}

async function handleMeetCallEnded(accountId: string, payload: MeetCallEndedPayload) {
  const session = activeMeetings.get(accountId);
  if (!session || session.code !== payload.code) {
    log('meet: call_ended with no matching session account=%s code=%s', accountId, payload.code);
    return;
  }
  activeMeetings.delete(accountId);
  await flushMeetingSession(accountId, session, payload.endedAt, payload.reason ?? 'unknown');
}

async function flushMeetingSession(
  accountId: string,
  session: MeetingSession,
  endedAt: number,
  reason: string
): Promise<void> {
  const segments = collapseToSegments(session.snapshots);
  const participants = new Set(segments.map(s => s.speaker));
  const markdown = renderTranscript(session, endedAt, segments, participants);

  const namespace = `google-meet:${accountId}`;
  const key = `${session.code}:${session.startedAt}`;
  const title = `Google Meet · ${session.code} · ${new Date(session.startedAt)
    .toISOString()
    .slice(0, 10)}`;

  try {
    await callCoreRpc({
      method: 'openhuman.memory_doc_ingest',
      params: {
        namespace,
        key,
        title,
        content: markdown,
        source_type: 'google-meet',
        priority: 'high',
        tags: ['google-meet', 'meeting-transcript', session.code],
        metadata: {
          provider: 'google-meet',
          account_id: accountId,
          meeting_code: session.code,
          started_at: session.startedAt,
          ended_at: endedAt,
          duration_s: Math.round((endedAt - session.startedAt) / 1000),
          participants: Array.from(participants),
          segment_count: segments.length,
          end_reason: reason,
        },
        category: 'core',
      },
    });
    log(
      'meet: persisted transcript account=%s code=%s segments=%d participants=%d',
      accountId,
      session.code,
      segments.length,
      participants.size
    );
    store.dispatch(
      appendLog({
        accountId,
        entry: {
          ts: endedAt,
          level: 'info',
          msg:
            segments.length === 0
              ? `[meet] ${session.code} ended — no captions captured (enable captions in Meet)`
              : `[meet] saved transcript for ${session.code} — ${segments.length} utterances, ${participants.size} speakers`,
        },
      })
    );

    if (segments.length > 0) {
      await handoffToOrchestrator(accountId, session, endedAt, markdown, participants);
    }
  } catch (err) {
    errLog('meet: memory write failed: %o', err);
    store.dispatch(
      appendLog({
        accountId,
        entry: {
          ts: endedAt,
          level: 'error',
          msg: `[meet] failed to save transcript for ${session.code}: ${err instanceof Error ? err.message : String(err)}`,
        },
      })
    );
  }
}

/**
 * After a meeting transcript is persisted, open a fresh thread with the
 * orchestrator agent and hand it the transcript so it can extract notes
 * (summary, decisions, action items) and proactively act on follow-ups.
 *
 * The orchestrator IS the LLM here — there's no separate summarisation
 * call. It produces structured notes inline as part of its reply and
 * routes any actionable items to its subagents/skills.
 */
async function handoffToOrchestrator(
  accountId: string,
  session: MeetingSession,
  endedAt: number,
  transcriptMarkdown: string,
  participants: Set<string>
): Promise<void> {
  const durationMin = Math.max(1, Math.round((endedAt - session.startedAt) / 60_000));
  const participantList = Array.from(participants).join(', ') || 'unknown participants';

  const prompt = [
    `I just finished a Google Meet call (\`${session.code}\`, ~${durationMin} min, with ${participantList}).`,
    '',
    'Please:',
    '1. Extract structured meeting notes — a brief summary, key decisions, action items (with owners + deadlines if mentioned), and open questions / follow-ups.',
    '2. For any action item that you can act on with your tools (drafting messages, scheduling follow-ups, creating tasks, updating notes, etc.), proactively handle it now and report back what you did.',
    '',
    'Transcript:',
    '',
    transcriptMarkdown,
  ].join('\n');

  try {
    const thread = await threadApi.createNewThread();
    log('meet: created orchestrator thread %s for code=%s', thread.id, session.code);
    await chatSend({ threadId: thread.id, message: prompt, model: MEET_ORCHESTRATOR_MODEL });
    log('meet: handed off to orchestrator thread=%s code=%s', thread.id, session.code);
    store.dispatch(
      appendLog({
        accountId,
        entry: {
          ts: endedAt,
          level: 'info',
          msg: `[meet] orchestrator working on notes + follow-ups for ${session.code} (thread ${thread.id})`,
        },
      })
    );
  } catch (err) {
    errLog('meet: orchestrator handoff failed: %o', err);
    store.dispatch(
      appendLog({
        accountId,
        entry: {
          ts: endedAt,
          level: 'error',
          msg: `[meet] failed to hand off ${session.code} to orchestrator: ${err instanceof Error ? err.message : String(err)}`,
        },
      })
    );
  }
}

/**
 * Collapse a sequence of caption snapshots into one segment per
 * continuous utterance per speaker.
 *
 * Meet's caption region renders a rolling text block per active
 * speaker: "Hi" → "Hi everyone" → "Hi everyone, welcome". Snapshots
 * come every ~2s. To recover discrete utterances we:
 *   1. Track an "active" state per speaker.
 *   2. When a snapshot's row extends the active text (prefix match or
 *      the active text is a suffix of the new one, covering Meet's
 *      periodic head-truncation) we keep the longer version.
 *   3. When a speaker's row disappears, OR the text jumps in a way
 *      that isn't an extension, commit the previous utterance and
 *      start a new one.
 *   4. At the end of the session commit anything still active.
 */
function collapseToSegments(snapshots: CaptionSnapshot[]): TranscriptSegment[] {
  const committed: TranscriptSegment[] = [];
  const active = new Map<string, { text: string; startTs: number; lastTs: number }>();

  const commit = (speaker: string, state: { text: string; startTs: number; lastTs: number }) => {
    const text = state.text.trim();
    if (!text) return;
    committed.push({ speaker, text, startTs: state.startTs, endTs: state.lastTs });
  };

  for (const snap of snapshots) {
    const seenThisSnap = new Set<string>();
    for (const row of snap.captions) {
      const speaker = (row.speaker || 'Unknown').trim() || 'Unknown';
      const text = (row.text || '').trim();
      if (!text) continue;
      seenThisSnap.add(speaker);

      const prev = active.get(speaker);
      if (!prev) {
        active.set(speaker, { text, startTs: snap.ts, lastTs: snap.ts });
        continue;
      }
      if (text.startsWith(prev.text)) {
        // Rolling forward — longer version of same utterance.
        prev.text = text;
        prev.lastTs = snap.ts;
      } else if (prev.text.endsWith(text) || prev.text.startsWith(text)) {
        // Same utterance, no new words this tick.
        prev.lastTs = snap.ts;
      } else {
        // Different utterance — commit the old one, start a new one.
        commit(speaker, prev);
        active.set(speaker, { text, startTs: snap.ts, lastTs: snap.ts });
      }
    }
    // Speakers whose caption row disappeared this snapshot → utterance done.
    for (const [speaker, state] of active.entries()) {
      if (!seenThisSnap.has(speaker)) {
        commit(speaker, state);
        active.delete(speaker);
      }
    }
  }
  for (const [speaker, state] of active.entries()) {
    commit(speaker, state);
  }

  committed.sort((a, b) => a.startTs - b.startTs);
  return committed;
}

function renderTranscript(
  session: MeetingSession,
  endedAt: number,
  segments: TranscriptSegment[],
  participants: Set<string>
): string {
  const startIso = new Date(session.startedAt).toISOString();
  const endIso = new Date(endedAt).toISOString();
  const durationMin = Math.round((endedAt - session.startedAt) / 60000);
  const parts =
    participants.size > 0
      ? Array.from(participants).sort()
      : ['(captions off or no speech detected)'];

  const header =
    `# Google Meet — ${session.code}\n` +
    `started: ${startIso}\n` +
    `ended: ${endIso}\n` +
    `duration: ${durationMin} min\n` +
    `participants: ${parts.join(', ')}\n` +
    `segments: ${segments.length}\n\n` +
    `## Transcript\n\n`;

  if (segments.length === 0) {
    return (
      header +
      '_No captions were captured during this meeting. Ensure "Turn on captions" is enabled in Meet for the live-transcript pipeline to produce output._\n'
    );
  }

  const lines = segments.map(seg => {
    const hhmm = new Date(seg.startTs).toISOString().slice(11, 19) + 'Z';
    return `**${seg.speaker}** ${hhmm}\n${seg.text}\n`;
  });

  return header + lines.join('\n');
}

interface OpenAccountArgs {
  accountId: string;
  provider: AccountProvider;
  bounds: { x: number; y: number; width: number; height: number };
}

export async function openWebviewAccount(args: OpenAccountArgs): Promise<void> {
  if (!isTauri()) throw new Error('webview accounts require the desktop app');
  log('open account=%s provider=%s', args.accountId, args.provider);
  store.dispatch(setAccountStatus({ accountId: args.accountId, status: 'pending' }));
  void ensureNotificationPermission();
  try {
    await invoke('webview_account_open', {
      args: { account_id: args.accountId, provider: args.provider, bounds: args.bounds },
    });
    store.dispatch(setAccountStatus({ accountId: args.accountId, status: 'open' }));
    void setFocusedAccount(args.accountId);
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    errLog('open failed: %s', msg);
    store.dispatch(
      setAccountStatus({ accountId: args.accountId, status: 'error', lastError: msg })
    );
    throw err;
  }
}

export async function setWebviewAccountBounds(
  accountId: string,
  bounds: { x: number; y: number; width: number; height: number }
): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke('webview_account_bounds', { args: { account_id: accountId, bounds } });
  } catch (err) {
    errLog('bounds failed: %o', err);
  }
}

export async function hideWebviewAccount(accountId: string): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke('webview_account_hide', { args: { account_id: accountId } });
  } catch (err) {
    errLog('hide failed: %o', err);
  }
}

export async function showWebviewAccount(accountId: string): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke('webview_account_show', { args: { account_id: accountId } });
  } catch (err) {
    errLog('show failed: %o', err);
  }
}

export async function closeWebviewAccount(accountId: string): Promise<void> {
  if (!isTauri()) return;
  log('close account=%s', accountId);
  await flushMeetingIfAny(accountId, 'webview-closed');
  try {
    await invoke('webview_account_close', { args: { account_id: accountId } });
    store.dispatch(setAccountStatus({ accountId, status: 'closed' }));
  } catch (err) {
    errLog('close failed: %o', err);
  }
}

/**
 * Close the webview and wipe its on-disk data directory so the provider
 * treats the next open as a fresh login. Use for user-initiated logout.
 */
export async function purgeWebviewAccount(accountId: string): Promise<void> {
  if (!isTauri()) return;
  log('purge account=%s', accountId);
  await flushMeetingIfAny(accountId, 'webview-purged');
  try {
    await invoke('webview_account_purge', { args: { account_id: accountId } });
    store.dispatch(setAccountStatus({ accountId, status: 'closed' }));
  } catch (err) {
    errLog('purge failed: %o', err);
    throw err;
  }
}

// ────────────────────────── Notification bypass helpers ─────────────────────

/**
 * Mute or unmute OS notification toasts for a specific embedded account.
 * When muted, toasts from that account are suppressed regardless of focus state.
 */
export async function setAccountMuted(accountId: string, muted: boolean): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke('webview_notification_mute_account', { accountId, muted });
    log('notify-bypass: account=%s muted=%s', accountId, muted);
  } catch (e) {
    log('notify-bypass: setAccountMuted error %o', e);
  }
}

/**
 * Enable or disable global Do Not Disturb mode for embedded webview notifications.
 * When enabled, all OS notification toasts from embedded accounts are suppressed.
 */
export async function setGlobalDnd(enabled: boolean): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke('webview_notification_set_dnd', { enabled });
    log('notify-bypass: global DND set to %s', enabled);
  } catch (e) {
    log('notify-bypass: setGlobalDnd error %o', e);
  }
}

/**
 * Fetch the current notification bypass preferences from the Rust side.
 * Returns `null` when not running in Tauri or on any invoke error.
 */
export async function getBypassPrefs(): Promise<{
  global_dnd: boolean;
  muted_accounts: string[];
  bypass_when_focused: boolean;
} | null> {
  if (!isTauri()) return null;
  try {
    return await invoke('webview_notification_get_bypass_prefs');
  } catch (e) {
    log('notify-bypass: getBypassPrefs error %o', e);
    return null;
  }
}

/**
 * Tell Rust which account (if any) the user is currently viewing.
 * Rust uses this together with the window-focus state to suppress
 * notifications while the user is actively looking at that account.
 */
export async function setFocusedAccount(accountId: string | null): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke('webview_set_focused_account', { accountId });
    log('notify-bypass: focused account set to %s', accountId);
  } catch (e) {
    log('notify-bypass: setFocusedAccount error %o', e);
  }
}

async function flushMeetingIfAny(accountId: string, reason: string): Promise<void> {
  const session = activeMeetings.get(accountId);
  if (!session) return;
  activeMeetings.delete(accountId);
  await flushMeetingSession(accountId, session, Date.now(), reason);
}
