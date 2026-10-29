import { type RefObject, useCallback, useContext, useEffect } from 'react';
import { type BlockerFunction, UNSAFE_DataRouterContext, useBlocker } from 'react-router-dom';

export interface BlockedFlowNavigation {
  proceed: () => void;
  reset: () => void;
}

interface FlowNavigationBlockerProps {
  dirty: boolean;
  /** Permit one deliberate navigation after save or confirmed discard. */
  bypassRef: RefObject<boolean>;
  blockedRef: RefObject<BlockedFlowNavigation | null>;
  onBlocked: () => void;
}

/**
 * Existing isolated component tests render under MemoryRouter, which has no
 * data-router blocking API. The real app uses createHashRouter, so block
 * navigation there while retaining the editor's local Back confirmation in
 * non-data-router embeddings.
 */
export default function FlowNavigationBlocker(props: FlowNavigationBlockerProps) {
  const dataRouter = useContext(UNSAFE_DataRouterContext);
  return dataRouter ? <ActiveFlowNavigationBlocker {...props} /> : null;
}

function ActiveFlowNavigationBlocker({
  dirty,
  bypassRef,
  blockedRef,
  onBlocked,
}: FlowNavigationBlockerProps) {
  const shouldBlock = useCallback<BlockerFunction>(
    ({ currentLocation, nextLocation }) => {
      // A bypass belongs to one attempted navigation. Never let it disable
      // future guards if a replace stays on this editor or a route is aborted.
      if (bypassRef.current) {
        bypassRef.current = false;
        return false;
      }
      return dirty && currentLocation.pathname !== nextLocation.pathname;
    },
    [dirty, bypassRef]
  );
  const blocker = useBlocker(shouldBlock);

  useEffect(() => {
    if (blocker.state !== 'blocked') {
      blockedRef.current = null;
      return;
    }

    const actions: BlockedFlowNavigation = {
      proceed: () => blocker.proceed?.(),
      reset: () => blocker.reset?.(),
    };
    blockedRef.current = actions;
    onBlocked();

    return () => {
      if (blockedRef.current === actions) blockedRef.current = null;
    };
  }, [blocker, blockedRef, onBlocked]);

  return null;
}
