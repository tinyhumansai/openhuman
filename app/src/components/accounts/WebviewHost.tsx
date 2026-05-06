import debug from 'debug';
import { useEffect, useRef, useState } from 'react';

import {
  hideWebviewAccount,
  openWebviewAccount,
  retryWebviewAccountLoad,
  setWebviewAccountBounds,
} from '../../services/webviewAccountService';
import { useAppSelector } from '../../store/hooks';
import type { AccountProvider, AccountStatus } from '../../types/accounts';
import { ProviderIcon } from './providerIcons';

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
  slack: 'Slack',
  discord: 'Discord',
  'google-meet': 'Google Meet',
  zoom: 'Zoom',
  browserscan: 'BrowserScan',
};

// Phase-hint thresholds for slow loads. Most cold opens finish well under
// 5s; the hints only render when something is actually taking a while so
// the wording never feels patronising on the happy path.
const PHASE_HINT_AT_MS = 5_000;
const PHASE_HINT_LATE_MS = 10_000;

/**
 * Counter-driven phase hint that escalates after 5s/10s of loading.
 *
 * Lives in its own component so the elapsed counter resets purely via
 * mount/unmount: `WebviewHost` only renders this child while the account
 * is in a loading state, so flipping out of `'loading'` unmounts it and
 * the next loading run starts fresh from zero. Keeps `WebviewHost`'s
 * effects free of synchronous `setState` calls (lint rule
 * `react-hooks/set-state-in-effect`) while preserving deterministic
 * fake-timer behaviour for tests — counter is incremented by an interval
 * tick rather than diffing `Date.now()`.
 */
const LoadingPhaseHint = ({ accountId }: { accountId: string }) => {
  const [elapsedMs, setElapsedMs] = useState(0);
  useEffect(() => {
    const tickMs = 500;
    const id = window.setInterval(() => {
      setElapsedMs(prev => prev + tickMs);
    }, tickMs);
    return () => window.clearInterval(id);
  }, []);
  const text =
    elapsedMs >= PHASE_HINT_LATE_MS
      ? 'Almost ready...'
      : elapsedMs >= PHASE_HINT_AT_MS
        ? 'Restoring session...'
        : null;
  if (!text) return null;
  return (
    <span
      data-testid={`webview-loading-hint-${accountId}`}
      className="text-[11px] font-medium text-stone-400">
      {text}
    </span>
  );
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
 *
 * Issue #1233 — to eliminate the perceived blank-screen gap before the
 * webview paints, the host always renders a branded placeholder (provider
 * icon + name) immediately on mount, with a spinner overlay while the
 * account is in a loading state. After 5s/10s the spinner adds a phase
 * hint so the user gets feedback that something is still happening.
 */
const WebviewHost = ({ accountId, provider }: WebviewHostProps) => {
  const ref = useRef<HTMLDivElement | null>(null);
  const lastBoundsRef = useRef<{ x: number; y: number; width: number; height: number } | null>(
    null
  );
  const openedRef = useRef(false);
  const status = useAppSelector(s => s.accounts.accounts[accountId]?.status);
  // Treat an unknown account status as "still loading" so the spinner is
  // visible from frame 1, even before the openWebviewAccount thunk has
  // dispatched setAccountStatus('pending'). The status flips out of the
  // loading set on the first 'open'/'timeout'/'closed' transition, so the
  // overlay never sticks beyond the actual load.
  const isLoading = status === undefined || LOADING_STATUSES.has(status);
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
      {/* Branded placeholder + (optional) loading overlay collapsed into a
          single absolute container so we never paint two stacked / offset
          flex columns when the spinner is on top of the placeholder.
          - Placeholder always rendered (icon + provider name) so the host
            area is never a blank stone-100 rectangle.
          - When loading: spinner + "Loading {Provider}..." appended below
            the same icon, plus the elapsed phase hint past 5s/10s.
          - Native CEF view composites above this on reveal, so the
            placeholder is only visible during the loading window. */}
      {!isTimeout ? (
        <div
          data-testid={`webview-placeholder-${accountId}`}
          className={`pointer-events-none absolute inset-0 flex flex-col items-center justify-center gap-3 ${
            isLoading ? 'text-stone-500' : 'text-stone-400'
          }`}
          role={isLoading ? 'status' : undefined}
          aria-live={isLoading ? 'polite' : undefined}
          aria-label={isLoading ? 'Loading account' : undefined}>
          <ProviderIcon
            provider={provider}
            className={`h-12 w-12 ${isLoading ? '' : 'opacity-70'}`}
          />
          <span
            className={`text-xs font-medium tracking-wide ${isLoading ? '' : 'text-stone-500'}`}>
            {isLoading ? `Loading ${providerName}...` : providerName}
          </span>
          {isLoading ? (
            <div
              data-testid={`webview-loading-${accountId}`}
              className="flex flex-col items-center gap-2">
              <div className="h-8 w-8 animate-spin rounded-full border-2 border-stone-300 border-t-stone-600" />
              {/* Issue #1233 — `key={accountId}` forces React to unmount the
                  hint when the user switches between two still-loading
                  accounts so the elapsed counter doesn't carry the
                  previous account's progress into the new one. */}
              <LoadingPhaseHint key={accountId} accountId={accountId} />
            </div>
          ) : null}
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
