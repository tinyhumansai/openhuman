import { beforeEach, describe, expect, it, vi } from 'vitest';

import { store } from '../../store';
import { addAccount, resetAccountsState } from '../../store/accountsSlice';
import { fetchRespondQueue } from '../../store/providerSurfaceSlice';
import { callCoreRpc } from '../coreRpcClient';
import { startWebviewAccountService, stopWebviewAccountService } from '../webviewAccountService';

type EventHandler = (evt: { payload: unknown }) => void;
const listeners = new Map<string, EventHandler>();

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn().mockResolvedValue(undefined),
  isTauri: vi.fn().mockReturnValue(true),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, handler: EventHandler) => {
    listeners.set(event, handler);
    return () => listeners.delete(event);
  }),
}));

vi.mock('../api/threadApi', () => ({ threadApi: { createNewThread: vi.fn() } }));
vi.mock('../chatService', () => ({ chatSend: vi.fn() }));
vi.mock('../coreRpcClient', () => ({ callCoreRpc: vi.fn().mockResolvedValue({}) }));
vi.mock('../notificationService', () => ({ ingestNotification: vi.fn() }));
vi.mock('../../utils/tauriCommands/common', () => ({ isTauri: vi.fn(() => true) }));
vi.mock('../../store/providerSurfaceSlice', async importOriginal => {
  const actual = await importOriginal<typeof import('../../store/providerSurfaceSlice')>();
  return {
    ...actual,
    fetchRespondQueue: vi.fn(() => ({ type: 'providerSurface/fetchRespondQueue' })),
  };
});

const ACCOUNT_ID = 'acct-discord-test';

async function fireEvent(
  kind: string,
  payload: Record<string, unknown>,
  provider = 'discord'
): Promise<void> {
  const handler = listeners.get('webview:event');
  if (!handler) throw new Error('webview:event listener not attached');
  handler({ payload: { account_id: ACCOUNT_ID, provider, kind, payload, ts: Date.now() } });
  await new Promise(r => setTimeout(r, 0));
}

describe('webviewAccountService — Discord events', () => {
  beforeEach(async () => {
    listeners.clear();
    vi.clearAllMocks();
    store.dispatch(resetAccountsState());
    store.dispatch(
      addAccount({
        id: ACCOUNT_ID,
        provider: 'discord',
        label: 'Discord',
        createdAt: new Date().toISOString(),
        status: 'closed',
      })
    );
    stopWebviewAccountService();
    startWebviewAccountService();
    await new Promise(r => setTimeout(r, 0));
  });

  it('does not persist raw discord ingest transport events to memory', async () => {
    await fireEvent('ingest', { source: 'cdp-ws', payload_data: '{"op":0}' });

    expect(callCoreRpc).not.toHaveBeenCalled();
    expect(fetchRespondQueue).not.toHaveBeenCalled();
  });

  it('persists generic non-discord ingest events through the legacy memory path', async () => {
    await fireEvent(
      'ingest',
      { snapshotKey: 'snap-1', unread: 2, messages: [{ sender: 'Telegram Alice', body: 'Ping' }] },
      'telegram'
    );

    expect(fetchRespondQueue).toHaveBeenCalledWith({ silent: true });
    expect(callCoreRpc).toHaveBeenCalledWith(
      expect.objectContaining({
        method: 'openhuman.memory_doc_ingest',
        params: expect.objectContaining({ namespace: 'webview:telegram:acct-discord-test' }),
      })
    );
    expect(store.getState().accounts.messages[ACCOUNT_ID]).toEqual([
      expect.objectContaining({ id: 'acct-discord-test:0', from: 'Telegram Alice', body: 'Ping' }),
    ]);
  });

  it('refreshes queue for normalized discord transcript events without re-writing memory', async () => {
    await fireEvent('discord_memory_ingest', {
      channelId: '123',
      channelName: 'general',
      messages: [{ sender: 'Alice', body: 'Ship it', date: 1715000000 }],
    });

    expect(fetchRespondQueue).toHaveBeenCalledWith({ silent: true });
    expect(callCoreRpc).not.toHaveBeenCalled();
    expect(store.getState().accounts.messages[ACCOUNT_ID]).toEqual([
      expect.objectContaining({
        id: 'acct-discord-test:0',
        from: 'Alice',
        body: 'Ship it',
        ts: 1715000000 * 1000,
      }),
    ]);
  });
});
