import { useState } from 'react';
import { type EventData, EVENTS, Joyride, STATUS } from 'react-joyride';

import { WALKTHROUGH_STEPS } from './walkthroughSteps';
import WalkthroughTooltip from './WalkthroughTooltip';

// ── localStorage keys ──────────────────────────────────────────────────────

const WALKTHROUGH_KEY = 'openhuman:walkthrough_completed';
const WALKTHROUGH_PENDING_KEY = 'openhuman:walkthrough_pending';

/**
 * Returns `true` when the walkthrough has been flagged as pending (the user
 * just finished onboarding) AND has not yet been completed or skipped.
 */
export function isWalkthroughPending(): boolean {
  return (
    localStorage.getItem(WALKTHROUGH_PENDING_KEY) === 'true' &&
    localStorage.getItem(WALKTHROUGH_KEY) !== 'true'
  );
}

/**
 * Flags the walkthrough as pending. Called by OnboardingLayout when the user
 * completes the wizard and is about to navigate to /home.
 */
export function setWalkthroughPending(): void {
  localStorage.setItem(WALKTHROUGH_PENDING_KEY, 'true');
  console.debug('[walkthrough] pending flag set');
}

/**
 * Marks the walkthrough as completed (or skipped). Once set, the walkthrough
 * will not show again.
 */
export function markWalkthroughComplete(): void {
  localStorage.setItem(WALKTHROUGH_KEY, 'true');
  localStorage.removeItem(WALKTHROUGH_PENDING_KEY);
  console.debug('[walkthrough] marked as complete');
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
const AppWalkthrough = () => {
  // Only start running if the walkthrough is pending on first render.
  // Using a lazy initializer keeps this stable across re-renders.
  const [run, setRun] = useState<boolean>(() => isWalkthroughPending());

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
        overlayColor: 'rgba(0, 0, 0, 0.35)',
        // Show back, primary (next/finish), and skip buttons in tooltip
        buttons: ['back', 'primary', 'skip'],
      }}
    />
  );
};

export default AppWalkthrough;
