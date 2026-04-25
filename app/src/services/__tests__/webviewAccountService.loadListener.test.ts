import { invoke } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { store } from '../../store';
import { addAccount, resetAccountsState } from '../../store/accountsSlice';
import {
  closeWebviewAccount,
  openWebviewAccount,
  setWebviewAccountBounds,
  startWebviewAccountService,
  stopWebviewAccountService,
} from '../webviewAccountService';

// Capture the handlers attached via `listen(...)` so tests can fire synthetic
// events and verify downstream behaviour without actually wiring Tauri IPC.
type EventHandler = (evt: { payload: unknown }) => void;
const listeners = new Map<string, EventHandler>();

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn().mockResolvedValue(undefined),
  isTauri: vi.fn().mockReturnValue(true),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, handler: EventHandler) => {
    listeners.set(event, handler);
    return () => {
      listeners.delete(event);
    };
  }),
}));

// The service pulls in heavy deps for unrelated flows (Meet transcript + core
// RPC). Stub them so the listener test doesn't drag the whole dependency graph
// through its setup.
vi.mock('../api/threadApi', () => ({ threadApi: { createNewThread: vi.fn() } }));
vi.mock('../chatService', () => ({ chatSend: vi.fn() }));
vi.mock('../coreRpcClient', () => ({ callCoreRpc: vi.fn() }));
vi.mock('../notificationService', () => ({ ingestNotification: vi.fn() }));

const ACCOUNT_ID = 'acct-123';

function seedAccount(): void {
  store.dispatch(resetAccountsState());
  store.dispatch(
    addAccount({
      id: ACCOUNT_ID,
      provider: 'telegram',
      label: 'Test',
      createdAt: new Date().toISOString(),
      status: 'closed',
    })
  );
}

async function fireLoadEvent(payload: { state: string; url?: string }): Promise<void> {
  const handler = listeners.get('webview-account:load');
  if (!handler) throw new Error('webview-account:load listener not attached');
  handler({ payload: { account_id: ACCOUNT_ID, url: '', ...payload } });
  // Drain to a macrotask so chained `.catch()` / `.then()` on the
  // `invoke()` promise inside the handler also settle before we assert.
  await new Promise(r => setTimeout(r, 0));
}

describe('webviewAccountService load listener', () => {
  beforeEach(async () => {
    listeners.clear();
    stopWebviewAccountService();
    // Tear down any per-account state left from the previous test (bounds
    // cache + loading flag) before re-arming the listener for this one.
    // `stopWebviewAccountService` already clears the module-level Maps;
    // `closeWebviewAccount` is the no-Tauri-side close path (the invoke is
    // mocked) and is here only as belt-and-braces.
    await closeWebviewAccount(ACCOUNT_ID);
    // Single mock reset so individual tests can rely on the `invoke`
    // resolved-value config they set up after this hook returns.
    vi.clearAllMocks();
    seedAccount();
    startWebviewAccountService();
  });

  it('transitions pending → loading on openWebviewAccount resolve', async () => {
    const bounds = { x: 10, y: 20, width: 800, height: 600 };
    await openWebviewAccount({ accountId: ACCOUNT_ID, provider: 'telegram', bounds });

    expect(store.getState().accounts.accounts[ACCOUNT_ID]?.status).toBe('loading');
    expect(vi.mocked(invoke)).toHaveBeenCalledWith(
      'webview_account_open',
      expect.objectContaining({
        args: expect.objectContaining({ account_id: ACCOUNT_ID, provider: 'telegram' }),
      })
    );
  });

  it('reveals with cached bounds + flips to open on finished event', async () => {
    const bounds = { x: 5, y: 15, width: 1024, height: 768 };
    await openWebviewAccount({ accountId: ACCOUNT_ID, provider: 'telegram', bounds });
    vi.mocked(invoke).mockClear();

    await fireLoadEvent({ state: 'finished', url: 'https://web.telegram.org/' });

    expect(vi.mocked(invoke)).toHaveBeenCalledWith('webview_account_reveal', {
      args: { account_id: ACCOUNT_ID, bounds },
    });
    expect(store.getState().accounts.accounts[ACCOUNT_ID]?.status).toBe('open');
  });

  it('reveals with latest bounds when resize landed during loading', async () => {
    const initial = { x: 0, y: 0, width: 800, height: 600 };
    await openWebviewAccount({ accountId: ACCOUNT_ID, provider: 'telegram', bounds: initial });

    // Resize during loading — invoke should be skipped, cache should still update.
    vi.mocked(invoke).mockClear();
    const resized = { x: 0, y: 0, width: 1200, height: 900 };
    await setWebviewAccountBounds(ACCOUNT_ID, resized);
    expect(vi.mocked(invoke)).not.toHaveBeenCalledWith('webview_account_bounds', expect.anything());

    await fireLoadEvent({ state: 'finished', url: 'x' });

    expect(vi.mocked(invoke)).toHaveBeenCalledWith('webview_account_reveal', {
      args: { account_id: ACCOUNT_ID, bounds: resized },
    });
  });

  it('forwards bounds once loading is done', async () => {
    const initial = { x: 0, y: 0, width: 800, height: 600 };
    await openWebviewAccount({ accountId: ACCOUNT_ID, provider: 'telegram', bounds: initial });
    await fireLoadEvent({ state: 'finished', url: 'x' });
    vi.mocked(invoke).mockClear();

    const next = { x: 10, y: 10, width: 900, height: 700 };
    await setWebviewAccountBounds(ACCOUNT_ID, next);

    expect(vi.mocked(invoke)).toHaveBeenCalledWith('webview_account_bounds', {
      args: { account_id: ACCOUNT_ID, bounds: next },
    });
  });

  it('still reveals on timeout fallback', async () => {
    const bounds = { x: 0, y: 0, width: 800, height: 600 };
    await openWebviewAccount({ accountId: ACCOUNT_ID, provider: 'telegram', bounds });
    vi.mocked(invoke).mockClear();

    await fireLoadEvent({ state: 'timeout', url: '' });

    expect(vi.mocked(invoke)).toHaveBeenCalledWith('webview_account_reveal', {
      args: { account_id: ACCOUNT_ID, bounds },
    });
    expect(store.getState().accounts.accounts[ACCOUNT_ID]?.status).toBe('open');
  });

  it('treats `reused` event as finished (warm re-open path)', async () => {
    const bounds = { x: 0, y: 0, width: 800, height: 600 };
    await openWebviewAccount({ accountId: ACCOUNT_ID, provider: 'telegram', bounds });
    vi.mocked(invoke).mockClear();

    await fireLoadEvent({ state: 'reused', url: 'https://web.telegram.org/' });

    expect(vi.mocked(invoke)).toHaveBeenCalledWith('webview_account_reveal', {
      args: { account_id: ACCOUNT_ID, bounds },
    });
    expect(store.getState().accounts.accounts[ACCOUNT_ID]?.status).toBe('open');
  });

  it('skips reveal when the account has already unmounted', async () => {
    // Fire load event without ever having opened the account (no cached bounds).
    vi.mocked(invoke).mockClear();

    await fireLoadEvent({ state: 'finished', url: 'x' });

    expect(vi.mocked(invoke)).not.toHaveBeenCalled();
  });
});
