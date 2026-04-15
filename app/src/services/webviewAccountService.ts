import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import debug from 'debug';

import { callCoreRpc } from './coreRpcClient';
import { store } from '../store';
import {
  appendLog,
  appendMessages,
  setAccountStatus,
} from '../store/accountsSlice';
import type { AccountProvider, IngestedMessage } from '../types/accounts';

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

let unlisten: UnlistenFn | null = null;
let started = false;

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
  })();
}

export function stopWebviewAccountService(): void {
  if (unlisten) {
    unlisten();
    unlisten = null;
  }
  started = false;
}

function handleRecipeEvent(evt: RecipeEventPayload) {
  const accountId = evt.account_id;
  if (!accountId) return;

  if (evt.kind === 'log') {
    const level = (evt.payload.level as 'info' | 'warn' | 'error' | 'debug') || 'info';
    const msg = String(evt.payload.msg ?? '');
    store.dispatch(
      appendLog({
        accountId,
        entry: { ts: evt.ts ?? Date.now(), level, msg },
      })
    );
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

    store.dispatch(
      appendMessages({
        accountId,
        messages,
        unread: ingest.unread,
      })
    );

    // Fire-and-forget memory write via the existing core RPC.
    // Namespace mirrors the skill-sync convention so the recall pipeline
    // can find these alongside other ingested context.
    void persistIngestToMemory(accountId, evt.provider, ingest, messages);
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

async function persistWhatsappChatDay(
  accountId: string,
  ingest: IngestPayload
): Promise<void> {
  const chatId = ingest.chatId as string;
  const day = ingest.day as string;
  const chatName = ingest.chatName ?? chatId;
  const raw = ingest.messages ?? [];
  if (raw.length === 0) return;

  // Stable namespace + key: same (chat, day) always upserts the same doc.
  const namespace = `whatsapp-web:${accountId}`;
  const key = `${chatId}:${day}`;

  const sorted = [...raw].sort(
    (a, b) => (a.timestamp ?? 0) - (b.timestamp ?? 0)
  );

  const transcriptLines = sorted.map(m => {
    const tsSec = m.timestamp ?? 0;
    const hhmm = tsSec
      ? new Date(tsSec * 1000).toISOString().slice(11, 16) + 'Z'
      : '--:--';
    const who = m.fromMe ? 'me' : (m.from ?? '?');
    const body = (m.body ?? '').replace(/\r?\n/g, ' ').trim();
    const kind = m.type && m.type !== 'chat' ? ` [${m.type}]` : '';
    return `[${hhmm}] ${who}${kind}: ${body}`;
  });

  const header = `# WhatsApp — ${chatName} — ${day}\n` +
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

interface OpenAccountArgs {
  accountId: string;
  provider: AccountProvider;
  bounds: { x: number; y: number; width: number; height: number };
}

export async function openWebviewAccount(args: OpenAccountArgs): Promise<void> {
  if (!isTauri()) throw new Error('webview accounts require the desktop app');
  log('open account=%s provider=%s', args.accountId, args.provider);
  store.dispatch(setAccountStatus({ accountId: args.accountId, status: 'pending' }));
  try {
    await invoke('webview_account_open', {
      args: {
        account_id: args.accountId,
        provider: args.provider,
        bounds: args.bounds,
      },
    });
    store.dispatch(setAccountStatus({ accountId: args.accountId, status: 'open' }));
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
  try {
    await invoke('webview_account_close', { args: { account_id: accountId } });
    store.dispatch(setAccountStatus({ accountId, status: 'closed' }));
  } catch (err) {
    errLog('close failed: %o', err);
  }
}
