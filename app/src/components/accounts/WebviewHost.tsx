import debug from 'debug';
import { useEffect, useRef } from 'react';

import {
  hideWebviewAccount,
  openWebviewAccount,
  retryWebviewAccountLoad,
  setWebviewAccountBounds,
} from '../../services/webviewAccountService';
import { useAppSelector } from '../../store/hooks';
import type { AccountProvider, AccountStatus } from '../../types/accounts';

const log = debug('webview-accounts:host');

interface WebviewHostProps {
  accountId: string;
  provider: AccountProvider;
}

const LOADING_STATUSES: ReadonlySet<AccountStatus> = new Set(['pending', 'loading']);

const PROVIDER_COPY: Record<AccountProvider, string> = {
  whatsapp: 'WhatsApp',
  telegram: 'Telegram',
  linkedin: 'LinkedIn',
  gmail: 'Gmail',
  slack: 'Slack',
  discord: 'Discord',
  'google-meet': 'Google Meet',
  zoom: 'Zoom',
  browserscan: 'BrowserScan',
};

/**
 * Reserves a rectangular slot in the React layout that the native child
 * webview is glued to. We measure the placeholder's bounding rect and
 * tell Rust to position the webview at the same spot. On unmount or
 * route change the webview is hidden (not destroyed) so its session
 * stays warm in the background.
 *
 * During the first-open cycle the CEF subview is parked off-screen by Rust so
 * the React loading overlay below isn't covered by an empty native view. The
 * overlay is dismissed when the `webview-account:load` event flips the account
 * status out of `pending`/`loading`.
 */
const WebviewHost = ({ accountId, provider }: WebviewHostProps) => {
  const ref = useRef<HTMLDivElement | null>(null);
  const lastBoundsRef = useRef<{ x: number; y: number; width: number; height: number } | null>(
    null
  );
  const openedRef = useRef(false);
  const status = useAppSelector(s => s.accounts.accounts[accountId]?.status);
  // Only render the spinner when the account is *actively* loading. We used
  // to also treat `status === undefined` as loading, but that meant a host
  // mounted for an account that's not in the store (e.g. a render race with
  // `addAccount`) would spin forever. The brief microtask between mount and
  // the `setAccountStatus('pending')` dispatch in `openWebviewAccount` is
  // visually indistinguishable from no overlay, so this is safe.
  const isLoading = status !== undefined && LOADING_STATUSES.has(status);
  const isTimeout = status === 'timeout';
  const providerName = PROVIDER_COPY[provider] ?? 'app';

  // Spawn / show + keep bounds synced on every layout change.
  // IMPORTANT: both refs are reset on cleanup so switching accountIds
  // (React reuses this component instance when only props change) does
  // not carry stale "already opened" / "last bounds" state into the next
  // account — otherwise the new webview either never spawns or the size
  // sync skips because the rect happens to match the previous account's.
  useEffect(() => {
    const el = ref.current;
    if (!el) return;

    openedRef.current = false;
    lastBoundsRef.current = null;

    let raf = 0;
    let cancelled = false;

    const measureAndSync = () => {
      if (!el || cancelled) return;
      const rect = el.getBoundingClientRect();
      // Inset the native webview by the container's border-radius so the
      // rounded HTML border is visible around the edges.
      const inset = 8;
      const bounds = {
        x: Math.round(rect.left + inset),
        y: Math.round(rect.top + inset),
        width: Math.max(1, Math.round(rect.width - inset * 2)),
        height: Math.max(1, Math.round(rect.height - inset * 2)),
      };
      const last = lastBoundsRef.current;
      const unchanged =
        last &&
        last.x === bounds.x &&
        last.y === bounds.y &&
        last.width === bounds.width &&
        last.height === bounds.height;

      // Always run the first open — even if measurement happened to
      // return identical bounds to a previous account, we still need to
      // create/show this one.
      if (unchanged && openedRef.current) return;
      lastBoundsRef.current = bounds;

      if (!openedRef.current) {
        openedRef.current = true;
        log('opening account=%s at %o', accountId, bounds);
        void openWebviewAccount({ accountId, provider, bounds });
      } else {
        void setWebviewAccountBounds(accountId, bounds);
      }
    };

    const scheduleMeasure = () => {
      if (raf) window.cancelAnimationFrame(raf);
      raf = window.requestAnimationFrame(measureAndSync);
    };

    scheduleMeasure();

    const ro = new ResizeObserver(scheduleMeasure);
    ro.observe(el);
    window.addEventListener('resize', scheduleMeasure);
    window.addEventListener('scroll', scheduleMeasure, true);

    return () => {
      cancelled = true;
      window.cancelAnimationFrame(raf);
      ro.disconnect();
      window.removeEventListener('resize', scheduleMeasure);
      window.removeEventListener('scroll', scheduleMeasure, true);
      openedRef.current = false;
      lastBoundsRef.current = null;
      void hideWebviewAccount(accountId);
    };
  }, [accountId, provider]);

  return (
    <div
      ref={ref}
      className="relative h-full w-full overflow-hidden rounded-2xl border border-stone-200/70 bg-stone-100 shadow-soft"
      aria-label={`webview host for account ${accountId}`}>
      {isLoading ? (
        <div
          data-testid={`webview-loading-${accountId}`}
          className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center gap-3 text-stone-500"
          role="status"
          aria-live="polite"
          aria-label="Loading account">
          <div className="h-8 w-8 animate-spin rounded-full border-2 border-stone-300 border-t-stone-600" />
          <span className="text-xs font-medium tracking-wide">{`Loading ${providerName}...`}</span>
        </div>
      ) : null}

      {isTimeout ? (
        <div
          data-testid={`webview-timeout-${accountId}`}
          className="absolute inset-0 z-10 flex flex-col items-center justify-center gap-4 bg-stone-50/95 px-6 text-center"
          role="status"
          aria-live="polite"
          aria-label="Webview load timeout">
          <div className="max-w-sm space-y-1">
            <p className="text-sm font-semibold text-stone-800">{`${providerName} is taking longer than expected.`}</p>
            <p className="text-xs text-stone-600">
              The embedded app may still be starting up. Retry to reload it without signing in
              again.
            </p>
          </div>
          <button
            type="button"
            onClick={() => {
              log('retry clicked account=%s provider=%s', accountId, provider);
              void retryWebviewAccountLoad(accountId, provider);
            }}
            className="rounded-md bg-primary-600 px-3 py-1.5 text-xs font-semibold text-white transition-colors hover:bg-primary-700">
            Retry loading
          </button>
        </div>
      ) : null}
    </div>
  );
};

export default WebviewHost;
