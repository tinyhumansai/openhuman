import * as Sentry from '@sentry/react';
import { invoke } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { store } from '../../store';
import { addAccount, resetAccountsState } from '../../store/accountsSlice';
import type { AccountProvider } from '../../types/accounts';
import {
  classifyWebviewAccountError,
  openWebviewAccount,
  retryWebviewAccountLoad,
  setWebviewAccountBounds,
  WebviewAccountError,
} from '../webviewAccountService';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
  isTauri: vi.fn().mockReturnValue(true),
}));

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => undefined) }));

vi.mock('@sentry/react', () => ({ addBreadcrumb: vi.fn() }));

// Heavy unrelated deps stubbed for the same reason as the listener test.
vi.mock('../api/threadApi', () => ({ threadApi: { createNewThread: vi.fn() } }));
vi.mock('../chatService', () => ({ chatSend: vi.fn() }));
vi.mock('../coreRpcClient', () => ({ callCoreRpc: vi.fn() }));
vi.mock('../notificationService', () => ({ ingestNotification: vi.fn() }));

const ACCOUNT_ID = 'acct-err-1';
const BOUNDS = { x: 0, y: 0, width: 800, height: 600 };

/**
 * Configure the Tauri `invoke` mock so that calls to `webview_account_open`
 * reject with `rejection`, while every other invoke (notification permission
 * probes, reveal, hide, bounds) resolves successfully. Without this the
 * `mockRejectedValueOnce` is consumed by `ensureNotificationPermission`'s
 * preflight invoke before openWebviewAccount ever calls the Rust opener.
 */
function rejectOpenWith(rejection: unknown): void {
  vi.mocked(invoke).mockImplementation(async (cmd: string) => {
    if (cmd === 'webview_account_open') {
      return Promise.reject(rejection);
    }
    return undefined;
  });
}

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

describe('classifyWebviewAccountError', () => {
  it.each([
    ['unknown provider: gmail', { kind: 'unknown_provider' as const, providerName: 'gmail' }],
    [
      'unknown provider: my-custom_provider.v2',
      { kind: 'unknown_provider' as const, providerName: 'my-custom_provider.v2' },
    ],
    ['no url for provider: foo', { kind: 'no_url' as const, providerName: 'foo' }],
    [
      'invalid provider url https://x: relative URL without a base',
      { kind: 'invalid_url' as const },
    ],
    ['something unrelated', { kind: 'unknown' as const }],
  ])('classifies %j', (message, expected) => {
    expect(classifyWebviewAccountError(message)).toEqual(expected);
  });
});

describe('openWebviewAccount error handling', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // `clearAllMocks` resets call history but NOT `mockImplementation`, so a
    // previous test's `rejectOpenWith(...)` would still apply here. Reset the
    // invoke implementation back to a benign default before every test.
    vi.mocked(invoke).mockReset();
    vi.mocked(invoke).mockResolvedValue(undefined);
    seedAccount();
  });

  it('wraps a raw string rejection from Tauri invoke into a typed WebviewAccountError', async () => {
    rejectOpenWith('unknown provider: gmail');

    await expect(
      openWebviewAccount({ accountId: ACCOUNT_ID, provider: 'telegram', bounds: BOUNDS })
    ).rejects.toBeInstanceOf(WebviewAccountError);

    // The rejection is no longer the bare string that Sentry's
    // `onunhandledrejection` handler captures as "Non-Error promise rejection".
    expect(store.getState().accounts.accounts[ACCOUNT_ID]?.status).toBe('error');
    // `lastError` must NOT carry the raw rejection text — that string can
    // include a user-supplied provider literal (debug-mode custom URL) so the
    // store keeps a fixed per-kind summary instead. Original message stays
    // attached to the thrown WebviewAccountError for internal control flow.
    expect(store.getState().accounts.accounts[ACCOUNT_ID]?.lastError).toBe(
      'Provider not supported'
    );
    expect(store.getState().accounts.accounts[ACCOUNT_ID]?.lastError).not.toContain(
      'unknown provider:'
    );
    expect(store.getState().accounts.accounts[ACCOUNT_ID]?.lastError).not.toContain('gmail');
  });

  it('exposes kind + providerName on the wrapped error', async () => {
    rejectOpenWith('unknown provider: slack');

    try {
      await openWebviewAccount({ accountId: ACCOUNT_ID, provider: 'slack', bounds: BOUNDS });
      throw new Error('should have rejected');
    } catch (err) {
      expect(err).toBeInstanceOf(WebviewAccountError);
      const wae = err as WebviewAccountError;
      expect(wae.kind).toBe('unknown_provider');
      expect(wae.providerName).toBe('slack');
    }
  });

  it('emits a Sentry breadcrumb with classifier output and no PII', async () => {
    rejectOpenWith('unknown provider: discord');

    await expect(
      openWebviewAccount({ accountId: ACCOUNT_ID, provider: 'discord', bounds: BOUNDS })
    ).rejects.toBeInstanceOf(WebviewAccountError);

    expect(Sentry.addBreadcrumb).toHaveBeenCalledWith(
      expect.objectContaining({
        category: 'webview-account',
        level: 'warning',
        message: 'webview_account_open rejected',
        data: expect.objectContaining({ kind: 'unknown_provider', provider: 'discord' }),
      })
    );

    // Breadcrumb must NOT carry the account id or the raw rejection text —
    // both could leak workspace identifiers / user state.
    const call = vi.mocked(Sentry.addBreadcrumb).mock.calls[0]?.[0];
    expect(JSON.stringify(call)).not.toContain(ACCOUNT_ID);
    expect(JSON.stringify(call)).not.toContain('unknown provider:');
  });

  it('classifies an unknown error string as kind="unknown" with error-level breadcrumb', async () => {
    rejectOpenWith('whatever the rust shell threw');

    await expect(
      openWebviewAccount({ accountId: ACCOUNT_ID, provider: 'telegram', bounds: BOUNDS })
    ).rejects.toMatchObject({ kind: 'unknown', providerName: undefined });

    expect(Sentry.addBreadcrumb).toHaveBeenCalledWith(
      expect.objectContaining({
        level: 'error',
        data: expect.objectContaining({ kind: 'unknown' }),
      })
    );
  });
});

describe('retryWebviewAccountLoad error handling', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // `clearAllMocks` resets call history but NOT `mockImplementation`, so a
    // previous test's `rejectOpenWith(...)` would still apply here. Reset the
    // invoke implementation back to a benign default before every test.
    vi.mocked(invoke).mockReset();
    vi.mocked(invoke).mockResolvedValue(undefined);
    seedAccount();
  });

  it('propagates a typed WebviewAccountError when invoke rejects', async () => {
    // Seed cached bounds via a successful open first (default mock resolves).
    await openWebviewAccount({ accountId: ACCOUNT_ID, provider: 'telegram', bounds: BOUNDS });

    // Now make the retry-time `webview_account_open` invoke fail.
    rejectOpenWith('unknown provider: gmail');

    await expect(
      retryWebviewAccountLoad(ACCOUNT_ID, 'gmail' as unknown as AccountProvider)
    ).rejects.toBeInstanceOf(WebviewAccountError);
  });

  it('no-ops silently when bounds were never cached', async () => {
    await expect(
      retryWebviewAccountLoad('never-opened', 'gmail' as unknown as AccountProvider)
    ).resolves.toBeUndefined();
    expect(invoke).not.toHaveBeenCalled();
  });

  // Defensive guard against the regression the parent ticket caught: a bare
  // `setWebviewAccountBounds(...)` invocation on an unknown account must not
  // surface as an unhandled rejection.
  it('setWebviewAccountBounds on a stale account does not throw', async () => {
    await expect(setWebviewAccountBounds('stale-id', BOUNDS)).resolves.toBeUndefined();
  });
});
