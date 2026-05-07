import { invoke } from '@tauri-apps/api/core';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { prewarmWebviewAccount } from '../webviewAccountService';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
  isTauri: vi.fn().mockReturnValue(true),
}));

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }));

vi.mock('../api/threadApi', () => ({ threadApi: { createNewThread: vi.fn() } }));
vi.mock('../chatService', () => ({ chatSend: vi.fn() }));
vi.mock('../coreRpcClient', () => ({ callCoreRpc: vi.fn() }));
vi.mock('../notificationService', () => ({ ingestNotification: vi.fn() }));

const ACCOUNT_ID = 'acct-prewarm-1';

describe('prewarmWebviewAccount (issue #1233)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('invokes the webview_account_prewarm Tauri command with snake_case args', async () => {
    vi.mocked(invoke).mockResolvedValueOnce(undefined);

    await prewarmWebviewAccount(ACCOUNT_ID, 'slack');

    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith('webview_account_prewarm', {
      args: { account_id: ACCOUNT_ID, provider: 'slack' },
    });
  });

  it('swallows backend errors so the caller never has to handle them', async () => {
    // Suppress the error log in test output.
    const errSpy = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    vi.mocked(invoke).mockRejectedValueOnce(new Error('add_child failed'));

    // Must not throw — prewarm is best-effort.
    await expect(prewarmWebviewAccount(ACCOUNT_ID, 'telegram')).resolves.toBeUndefined();
    expect(invoke).toHaveBeenCalledWith('webview_account_prewarm', {
      args: { account_id: ACCOUNT_ID, provider: 'telegram' },
    });
    errSpy.mockRestore();
  });

  it('is a no-op when not running inside Tauri', async () => {
    const { isTauri } = await import('@tauri-apps/api/core');
    vi.mocked(isTauri).mockReturnValueOnce(false);

    await prewarmWebviewAccount(ACCOUNT_ID, 'google-meet');

    expect(invoke).not.toHaveBeenCalled();
  });
});
