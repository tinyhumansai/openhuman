import { useCallback, useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { useLocation, useNavigate } from 'react-router-dom';

import Onboarding from '../pages/onboarding/Onboarding';
import { useCoreState } from '../providers/CoreStateProvider';
import { DEV_FORCE_ONBOARDING } from '../utils/config';

/**
 * Full-screen overlay that renders the onboarding flow on top of any page
 * when the user has not completed onboarding.
 *
 * Reads `onboarding_completed` from the core config (persisted in config.toml).
 */
const OnboardingOverlay = () => {
  const location = useLocation();
  const navigate = useNavigate();
  const { isBootstrapping, snapshot, setOnboardingCompletedFlag } = useCoreState();
  const token = snapshot.sessionToken;
  // Keep the overlay rendered while navigating away so the home page doesn't flash.
  const [isDismissing, setIsDismissing] = useState(false);
  const onboardingCompleted = snapshot.onboardingCompleted;

  const handleDone = useCallback(async () => {
    // Navigate first while the overlay is still covering the screen, then
    // persist the completed flag. This prevents the home page from flashing
    // between overlay dismissal and route change.
    setIsDismissing(true);
    console.debug('[onboarding:overlay] completion finished; navigating to chat');
    navigate('/chat', { replace: true });
    try {
      await setOnboardingCompletedFlag(true);
      console.debug('[onboarding:overlay] persisted onboarding_completed=true');
    } catch {
      console.warn('[onboarding] Failed to persist onboarding_completed');
    }
  }, [setOnboardingCompletedFlag, navigate]);

  useEffect(() => {
    if (!isDismissing) return;
    if (location.pathname === '/chat') {
      console.debug('[onboarding:overlay] chat active; dismissing transition mask');
      setIsDismissing(false);
    }
  }, [isDismissing, location.pathname]);

  useEffect(() => {
    setIsDismissing(false);
  }, [token]);

  // Don't show if not logged in or bootstrap not complete.
  // Showing immediately after bootstrap removes a first-launch delay.
  if (!token || isBootstrapping) return null;

  const shouldShow = isDismissing || DEV_FORCE_ONBOARDING || !onboardingCompleted;

  if (!shouldShow) return null;

  return createPortal(
    <div className="fixed inset-0 z-[9999] bg-white/50 backdrop-blur-md flex items-center justify-center">
      <Onboarding onComplete={handleDone} onDefer={handleDone} />
    </div>,
    document.body
  );
};

export default OnboardingOverlay;
