import { useCallback, useEffect, useState } from 'react';
import { createPortal } from 'react-dom';

import Onboarding from '../pages/onboarding/Onboarding';
import { selectIsOnboarded, selectOnboardingDeferred } from '../store/authSelectors';
import { setOnboardingDeferredForUser } from '../store/authSlice';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import { DEV_FORCE_ONBOARDING } from '../utils/config';
import {
  DEFAULT_WORKSPACE_ONBOARDING_FLAG,
  openhumanWorkspaceOnboardingFlagExists,
} from '../utils/tauriCommands';

/**
 * Full-screen overlay that renders the onboarding flow on top of any page
 * when the user has not completed onboarding.
 *
 * Checks both Redux `isOnboarded` and the workspace flag file.
 * Waits for the user profile to load before making a decision.
 */
const OnboardingOverlay = () => {
  const dispatch = useAppDispatch();
  const token = useAppSelector(state => state.auth.token);
  const isAuthBootstrapComplete = useAppSelector(state => state.auth.isAuthBootstrapComplete);
  const user = useAppSelector(state => state.user.user);
  const isOnboarded = useAppSelector(selectIsOnboarded);
  const isDeferred = useAppSelector(selectOnboardingDeferred);
  // Fallback: if we have a token and any user was previously onboarded,
  // don't flash the overlay while waiting for the user profile to load.
  const anyUserOnboarded = useAppSelector(
    state => Object.values(state.auth.isOnboardedByUser).some(Boolean)
  );
  const [hasWorkspaceFlag, setHasWorkspaceFlag] = useState<boolean | null>(null);
  const [dismissed, setDismissed] = useState(false);
  const [userLoadTimedOut, setUserLoadTimedOut] = useState(false);

  // Timeout: if user profile hasn't loaded after 3s but we have token + bootstrap,
  // proceed anyway so onboarding isn't permanently invisible.
  useEffect(() => {
    if (!token || !isAuthBootstrapComplete || user?._id) return;

    const timer = setTimeout(() => setUserLoadTimedOut(true), 3000);
    return () => clearTimeout(timer);
  }, [token, isAuthBootstrapComplete, user?._id]);

  // User is ready when profile loaded or timeout elapsed.
  // Note: userLoadTimedOut is sticky across sessions but harmless — when token
  // is null (logged out) the early-return guard prevents any visible effect,
  // and the workspace flag check doesn't require userId.
  const userReady = !!user?._id || userLoadTimedOut;
  useEffect(() => {
    if (!token || !isAuthBootstrapComplete || !userReady) return;

    let mounted = true;
    const check = async () => {
      try {
        const exists = await openhumanWorkspaceOnboardingFlagExists(
          DEFAULT_WORKSPACE_ONBOARDING_FLAG
        );
        if (mounted) setHasWorkspaceFlag(exists);
      } catch {
        if (mounted) setHasWorkspaceFlag(false);
      }
    };
    void check();
    return () => {
      mounted = false;
    };
  }, [token, isAuthBootstrapComplete, userReady, isOnboarded]);

  const handleComplete = useCallback(() => {
    setDismissed(true);
  }, []);

  const handleDefer = useCallback(() => {
    if (user?._id) {
      dispatch(setOnboardingDeferredForUser({ userId: user._id, deferred: true }));
    }
    setDismissed(true);
  }, [dispatch, user]);

  // Don't show if not logged in, bootstrap not complete, or user not ready
  if (!token || !isAuthBootstrapComplete || !userReady) return null;

  // Still loading workspace flag
  if (hasWorkspaceFlag === null) return null;

  // Determine if onboarding should show.
  // anyUserOnboarded prevents the overlay from flashing while the user profile
  // is still loading (selectIsOnboarded returns false when user._id is null).
  const shouldShow = DEV_FORCE_ONBOARDING
    ? !dismissed
    : !isOnboarded && !anyUserOnboarded && !hasWorkspaceFlag && !isDeferred && !dismissed;

  if (!shouldShow) return null;

  return createPortal(
    <div className="fixed inset-0 z-[9999] bg-canvas-900/95 backdrop-blur-md flex items-center justify-center">
      <Onboarding onComplete={handleComplete} onDefer={handleDefer} />
    </div>,
    document.body
  );
};

export default OnboardingOverlay;
