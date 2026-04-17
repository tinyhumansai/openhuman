import debug from 'debug';
import { useEffect, useRef } from 'react';

import {
  hideWebviewAccount,
  openWebviewAccount,
  setWebviewAccountBounds,
} from '../../services/webviewAccountService';
import type { AccountProvider } from '../../types/accounts';

const log = debug('webview-accounts:host');

interface WebviewHostProps {
  accountId: string;
  provider: AccountProvider;
}

/**
 * Reserves a rectangular slot in the React layout that the native child
 * webview is glued to. We measure the placeholder's bounding rect and
 * tell Rust to position the webview at the same spot. On unmount or
 * route change the webview is hidden (not destroyed) so its session
 * stays warm in the background.
 */
const WebviewHost = ({ accountId, provider }: WebviewHostProps) => {
  const ref = useRef<HTMLDivElement | null>(null);
  const lastBoundsRef = useRef<{ x: number; y: number; width: number; height: number } | null>(
    null
  );
  const openedRef = useRef(false);

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
      const bounds = {
        x: Math.round(rect.left),
        y: Math.round(rect.top),
        width: Math.max(1, Math.round(rect.width)),
        height: Math.max(1, Math.round(rect.height)),
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
      className="relative h-full w-full overflow-hidden rounded-lg border border-stone-200 bg-stone-100"
      aria-label={`webview host for account ${accountId}`}
    />
  );
};

export default WebviewHost;
