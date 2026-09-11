import debugFactory from 'debug';
import { useEffect } from 'react';

import { useCoreState } from '../../../providers/CoreStateProvider';
import { isLocalSessionToken } from '../../../utils/localSession';

const log = debugFactory('sidebar');

/**
 * Whether cloud-only nav entries (today: Rewards) should be offered.
 *
 * Credits, referrals and coupons live behind the backend rewards API, and the
 * page itself renders an "unavailable" state for a local session, so there is
 * no point offering the row there.
 *
 * The `isReady` term is load-bearing, not defensive: the initial core snapshot
 * is `{ isReady: false, sessionToken: null }`, and `isLocalSessionToken(null)`
 * is `false` — so gating on the token alone briefly *shows* the entry to a
 * local session until the first refresh resolves, then yanks it away.
 *
 * Extracted from `AppSidebar` when Rewards moved from the sidebar footer into
 * the primary nav: `SidebarNav` and `CollapsedNavRail` both render `NAV_TABS`
 * and both have to apply the same gate, and a second hand-rolled copy of that
 * three-term condition is exactly how the two rails drift apart.
 */
export function useCloudNavGate(): boolean {
  const { snapshot, isReady } = useCoreState();
  const allowed =
    isReady && Boolean(snapshot.sessionToken) && !isLocalSessionToken(snapshot.sessionToken);

  // Log the gate outcome whenever it resolves/flips. Booleans only — never the
  // session token or a raw path.
  useEffect(() => {
    log(
      'cloud nav gate resolved: allowed=%s isReady=%s hasSession=%s local=%s',
      allowed,
      isReady,
      Boolean(snapshot.sessionToken),
      isLocalSessionToken(snapshot.sessionToken)
    );
  }, [allowed, isReady, snapshot.sessionToken]);

  return allowed;
}
