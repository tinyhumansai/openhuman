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
  /**
   * When `true` and the user reaches `onConnected`, the modal hides
   * itself but does NOT purge the underlying webview. The parent owns
   * the lifecycle — typically because it wants to keep driving the
   * webview via CDP from the next onboarding step (e.g. running
   * Gmail search for the LinkedIn-enrichment pipeline). On cancel we
   * still purge regardless, so abandoned sessions don't leak.
   */
  keepAliveOnConnected?: boolean;
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

interface NavigatePayload {
  account_id: string;
  provider: string;
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
const WebviewLoginModal = ({
  provider,
  label,
  onConnected,
  onClose,
  keepAliveOnConnected = false,
}: WebviewLoginModalProps) => {
  const dispatch = useAppDispatch();
  const accountId = useMemo(() => makeAccountId(), []);
  const [autoDetected, setAutoDetected] = useState(false);
  const onConnectedRef = useRef(onConnected);
  onConnectedRef.current = onConnected;
  // Set whenever the user reaches the connected state. Read in cleanup
  // to decide whether the webview should be purged (cancel) or kept
  // alive for the next onboarding step (success + keepAliveOnConnected).
  const connectedRef = useRef(false);

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
      const shouldPurge = !(keepAliveOnConnected && connectedRef.current);
      console.debug('[onboarding:webview-login] tearing down', { accountId, purge: shouldPurge });
      if (shouldPurge) {
        // Best-effort: purge the embedded webview and drop the account
        // from the store so cancel/close doesn't leave a zombie behind.
        void purgeWebviewAccount(accountId).catch(() => undefined);
        dispatch(removeAccount({ accountId }));
      }
      // When keeping alive, the parent now owns the account — leave
      // store + webview intact. The parent must call purge when done.
    };
  }, [accountId, provider, label, dispatch, keepAliveOnConnected]);

  // Listen for `webview-account:load` events scoped to *this* accountId
  // and fire onConnected once the URL crosses into the logged-in surface.
  useEffect(() => {
    let unlistenFn: UnlistenFn | null = null;
    let cancelled = false;

    // Tauri's `listen()` resolves asynchronously; under React StrictMode
    // (mount → cleanup → mount) the cleanup can run before the promise
    // settles, leaving us racing the resolution. The Tauri IPC layer
    // throws "Cannot read properties of undefined (reading 'handlerId')"
    // when the same handler is unregistered twice (open → close → open
    // re-runs cleanup against a handle that was already invalidated by
    // the previous teardown).
    //
    // The unlisten function is typed `() => void` but Tauri's
    // implementation actually returns a Promise that can reject — that's
    // why the error surfaces as "Uncaught (in promise)" rather than a
    // sync throw. Wrap both paths and discard the rejection.
    const safeUnlisten = (fn: UnlistenFn | null) => {
      if (!fn) return;
      try {
        const result = fn() as unknown;
        if (result && typeof (result as Promise<unknown>).then === 'function') {
          (result as Promise<unknown>).catch(err => {
            console.debug('[onboarding:webview-login] unlisten rejected', err);
          });
        }
      } catch (err) {
        console.debug('[onboarding:webview-login] unlisten threw', err);
      }
    };

    const matchesLoggedIn = (url: string | undefined) => {
      const prefix = LOGGED_IN_URL_PREFIX[provider];
      return Boolean(prefix && url && url.startsWith(prefix));
    };

    let unlistenLoad: UnlistenFn | null = null;
    let unlistenNavigate: UnlistenFn | null = null;

    (async () => {
      try {
        // `webview-account:load` only fires once per cold open (the Rust
        // side dedups), so it catches the case where Gmail loads
        // straight into the inbox (already logged in).
        const onLoad = await listen<LoadPayload>('webview-account:load', evt => {
          const payload = evt.payload;
          if (!payload || payload.account_id !== accountId) return;
          console.debug('[onboarding:webview-login] load event', {
            accountId,
            url: payload.url,
            state: payload.state,
          });
          if (matchesLoggedIn(payload.url)) setAutoDetected(true);
        });
        // `webview-account:navigate` fires for every committed
        // navigation, so it catches the post-login redirect
        // (accounts.google.com → mail.google.com).
        const onNavigate = await listen<NavigatePayload>('webview-account:navigate', evt => {
          const payload = evt.payload;
          if (!payload || payload.account_id !== accountId) return;
          console.debug('[onboarding:webview-login] navigate event', {
            accountId,
            url: payload.url,
          });
          if (matchesLoggedIn(payload.url)) setAutoDetected(true);
        });
        if (cancelled) {
          safeUnlisten(onLoad);
          safeUnlisten(onNavigate);
        } else {
          unlistenLoad = onLoad;
          unlistenNavigate = onNavigate;
          unlistenFn = () => {
            safeUnlisten(unlistenLoad);
            safeUnlisten(unlistenNavigate);
            unlistenLoad = null;
            unlistenNavigate = null;
          };
        }
      } catch (err) {
        console.warn('[onboarding:webview-login] failed to attach listeners', err);
      }
    })();

    return () => {
      cancelled = true;
      const fn = unlistenFn;
      unlistenFn = null;
      safeUnlisten(fn);
    };
  }, [accountId, provider]);

  // Auto-advance once login is detected.
  useEffect(() => {
    if (autoDetected) {
      connectedRef.current = true;
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
              onClick={() => {
                connectedRef.current = true;
                onConnected(accountId);
              }}
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
        {/* `min-h-0` is required: WebviewHost reads its bounds from
            `getBoundingClientRect()`, and a flex child with the default
            `min-height: auto` collapses the inner `h-full` to 0 in some
            layouts — the CEF subview then spawns at 1×1px and never
            resizes. Forcing `min-h-0` lets `flex-1` actually shrink/grow. */}
        <div className="relative flex-1 min-h-0 min-w-0 bg-stone-50">
          <WebviewHost accountId={accountId} provider={provider} />
        </div>
      </div>
    </div>
  );
};

export default WebviewLoginModal;
