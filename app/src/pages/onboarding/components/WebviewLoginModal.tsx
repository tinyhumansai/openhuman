import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { useEffect, useMemo, useRef, useState } from 'react';

import WebviewHost from '../../../components/accounts/WebviewHost';
import { purgeWebviewAccount } from '../../../services/webviewAccountService';
import { addAccount, removeAccount } from '../../../store/accountsSlice';
import { useAppDispatch } from '../../../store/hooks';
import type { Account, AccountProvider } from '../../../types/accounts';

interface WebviewLoginModalProps {
  provider: AccountProvider;
  label: string;
  /** Called once we detect a successful sign-in. */
  onConnected: (accountId: string) => void;
  onClose: () => void;
}

/**
 * URL prefix that signals the user reached the post-login surface for a
 * given provider. Login flows (e.g. accounts.google.com) load *before*
 * we land here, so a `webview-account:load` whose URL starts with this
 * prefix is treated as "logged in".
 */
const LOGGED_IN_URL_PREFIX: Partial<Record<AccountProvider, string>> = {
  gmail: 'https://mail.google.com/',
};

interface LoadPayload {
  account_id: string;
  state: string;
  url: string;
}

function makeAccountId(): string {
  const c = globalThis.crypto;
  if (c && typeof c.randomUUID === 'function') return c.randomUUID();
  return `acct-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

/**
 * Onboarding webview login modal. Spawns an embedded webview for the
 * given provider and resolves once the page navigates to the provider's
 * logged-in URL. The user can also click "I'm signed in" to confirm
 * manually if the auto-detector misses.
 */
const WebviewLoginModal = ({ provider, label, onConnected, onClose }: WebviewLoginModalProps) => {
  const dispatch = useAppDispatch();
  const accountId = useMemo(() => makeAccountId(), []);
  const [autoDetected, setAutoDetected] = useState(false);
  const onConnectedRef = useRef(onConnected);
  onConnectedRef.current = onConnected;

  // Spawn the webview on mount and tear it down on unmount.
  useEffect(() => {
    const acct: Account = {
      id: accountId,
      provider,
      label,
      createdAt: new Date().toISOString(),
      status: 'pending',
    };
    dispatch(addAccount(acct));
    console.debug('[onboarding:webview-login] account created', { accountId, provider });

    return () => {
      console.debug('[onboarding:webview-login] tearing down webview account', { accountId });
      // Best-effort: purge the embedded webview and drop the account from
      // the store so cancel/close doesn't leave a zombie behind.
      void purgeWebviewAccount(accountId).catch(() => undefined);
      dispatch(removeAccount({ accountId }));
    };
  }, [accountId, provider, label, dispatch]);

  // Listen for `webview-account:load` events scoped to *this* accountId
  // and fire onConnected once the URL crosses into the logged-in surface.
  useEffect(() => {
    let unlistenFn: UnlistenFn | null = null;
    let cancelled = false;
    (async () => {
      try {
        unlistenFn = await listen<LoadPayload>('webview-account:load', evt => {
          const payload = evt.payload;
          if (!payload || payload.account_id !== accountId) return;
          const prefix = LOGGED_IN_URL_PREFIX[provider];
          console.debug('[onboarding:webview-login] load event', {
            accountId,
            url: payload.url,
            state: payload.state,
          });
          if (prefix && payload.url && payload.url.startsWith(prefix)) {
            setAutoDetected(true);
          }
        });
      } catch (err) {
        console.warn('[onboarding:webview-login] failed to attach load listener', err);
      }
      if (cancelled && unlistenFn) {
        unlistenFn();
        unlistenFn = null;
      }
    })();
    return () => {
      cancelled = true;
      if (unlistenFn) unlistenFn();
    };
  }, [accountId, provider]);

  // Auto-advance once login is detected.
  useEffect(() => {
    if (autoDetected) {
      onConnectedRef.current(accountId);
    }
  }, [autoDetected, accountId]);

  return (
    <div
      className="fixed inset-0 z-[10000] flex items-center justify-center bg-black/50 backdrop-blur-sm"
      role="dialog"
      aria-modal="true">
      <div className="flex h-[80vh] w-[min(960px,92vw)] flex-col overflow-hidden rounded-2xl bg-white shadow-strong">
        <div className="flex items-center justify-between border-b border-stone-100 px-4 py-3">
          <div className="flex items-center gap-2">
            <h2 className="text-sm font-semibold text-stone-900">Sign in to {label}</h2>
            <span className="text-xs text-stone-500">Your credentials stay in your device.</span>
          </div>
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => onConnected(accountId)}
              className="rounded-lg border border-sage-200 bg-sage-50 px-3 py-1.5 text-xs font-medium text-sage-700 hover:bg-sage-100 transition-colors">
              I'm already signed in
            </button>
            <button
              type="button"
              onClick={onClose}
              className="rounded-lg border border-stone-200 bg-white px-3 py-1.5 text-xs font-medium text-stone-700 hover:bg-stone-100 transition-colors"
              aria-label="Cancel">
              Cancel
            </button>
          </div>
        </div>
        <div className="relative flex-1 bg-stone-50">
          <WebviewHost accountId={accountId} provider={provider} />
        </div>
      </div>
    </div>
  );
};

export default WebviewLoginModal;
