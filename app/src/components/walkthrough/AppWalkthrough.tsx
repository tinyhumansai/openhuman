import { useState } from 'react';
import { type EventData, EVENTS, Joyride, STATUS } from 'react-joyride';

import { WALKTHROUGH_STEPS } from './walkthroughSteps';
import WalkthroughTooltip from './WalkthroughTooltip';

// ── localStorage keys ──────────────────────────────────────────────────────

const WALKTHROUGH_KEY = 'openhuman:walkthrough_completed';
const WALKTHROUGH_PENDING_KEY = 'openhuman:walkthrough_pending';

/**
 * Returns `true` when the walkthrough should be shown. This is true when:
 *  - The walkthrough has not yet been completed or skipped, AND
 *  - Either the pending flag was explicitly set (fresh onboarding), OR
 *    the caller indicates the user is already onboarded (migration path
 *    for existing users who upgrade to the Joyride version).
 *
 * Wrapped in try/catch to gracefully handle SecurityError or quota exceptions
 * (e.g., in private-browsing mode or when storage is full/blocked).
 */
export function isWalkthroughPending(userIsOnboarded = false): boolean {
  try {
    if (localStorage.getItem(WALKTHROUGH_KEY) === 'true') return false;
    return localStorage.getItem(WALKTHROUGH_PENDING_KEY) === 'true' || userIsOnboarded;
  } catch (e) {
    console.warn('[walkthrough] localStorage unavailable — treating as not pending', e);
    return false;
  }
}

/**
 * Flags the walkthrough as pending. Called by OnboardingLayout when the user
 * completes the wizard and is about to navigate to /home.
 *
 * Best-effort: if localStorage is unavailable (SecurityError / quota) the
 * error is logged and the call is silently swallowed so navigation always
 * proceeds.
 */
export function setWalkthroughPending(): void {
  try {
    localStorage.setItem(WALKTHROUGH_PENDING_KEY, 'true');
    console.debug('[walkthrough] pending flag set');
  } catch (e) {
    console.warn('[walkthrough] could not set pending flag in localStorage', e);
  }
}

/**
 * Marks the walkthrough as completed (or skipped). Once set, the walkthrough
 * will not show again.
 *
 * Wrapped in try/catch to prevent SecurityError/quota exceptions from
 * interrupting the tour-end flow.
 */
export function markWalkthroughComplete(): void {
  try {
    localStorage.setItem(WALKTHROUGH_KEY, 'true');
    localStorage.removeItem(WALKTHROUGH_PENDING_KEY);
    console.debug('[walkthrough] marked as complete');
  } catch (e) {
    console.warn('[walkthrough] could not mark walkthrough complete in localStorage', e);
  }
}

// ── Component ──────────────────────────────────────────────────────────────

/**
 * Renders the post-onboarding Joyride walkthrough overlay (react-joyride v3).
 *
 * Only mounts the Joyride instance when `isWalkthroughPending()` is true.
 * On finish or skip (EVENTS.TOUR_END), calls `markWalkthroughComplete()` so
 * it never shows again.
 *
 * Mount this inside the Home page so it runs after the tab bar and home card
 * are in the DOM (all `data-walkthrough="*"` targets must exist).
 */
const AppWalkthrough = ({ onboarded = false }: { onboarded?: boolean }) => {
  // Only start running if the walkthrough is pending on first render.
  // Using a lazy initializer keeps this stable across re-renders.
  const [run, setRun] = useState<boolean>(() => isWalkthroughPending(onboarded));

  const handleEvent = (data: EventData) => {
    const { type, status } = data;
    console.debug('[walkthrough] event', { type, status, index: data.index });

    // TOUR_END fires when the tour finishes or is skipped.
    if (type === EVENTS.TOUR_END) {
      if (status === STATUS.FINISHED || status === STATUS.SKIPPED) {
        markWalkthroughComplete();
        setRun(false);
      }
    }
  };

  // Nothing to render when the walkthrough is not pending.
  if (!run) return null;

  return (
    <Joyride
      steps={WALKTHROUGH_STEPS}
      run={run}
      continuous={true}
      tooltipComponent={WalkthroughTooltip}
      onEvent={handleEvent}
      options={{
        zIndex: 1200,
        overlayColor: 'rgba(0, 0, 0, 0.4)',
        buttons: ['back', 'primary', 'skip'],
        spotlightRadius: 16,
        spotlightPadding: 8,
      }}
    />
  );
};

export default AppWalkthrough;
