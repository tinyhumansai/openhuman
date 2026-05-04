/**
 * Tests for the Joyride walkthrough components introduced in #1123.
 *
 * Verifies:
 *  - isWalkthroughPending / setWalkthroughPending / markWalkthroughComplete helpers
 *  - AppWalkthrough renders only when pending
 *  - AppWalkthrough does not render when already completed
 *  - Completing/skipping the tour sets localStorage correctly
 *  - Step count matches WALKTHROUGH_STEPS
 */
import { render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  isWalkthroughPending,
  markWalkthroughComplete,
  setWalkthroughPending,
} from '../AppWalkthrough';
import { WALKTHROUGH_STEPS } from '../walkthroughSteps';

// ── Mock react-joyride so tests don't need a real DOM with
//    positioned elements for each step target. ─────────────────────────────

vi.mock('react-joyride', () => ({
  Joyride: ({ run }: { run: boolean }) => <div data-testid="joyride-mock" data-run={String(run)} />,
  EVENTS: { TOUR_END: 'tour:end' },
  STATUS: { FINISHED: 'finished', SKIPPED: 'skipped' },
}));

// ── localStorage helpers ───────────────────────────────────────────────────

const WALKTHROUGH_KEY = 'openhuman:walkthrough_completed';
const WALKTHROUGH_PENDING_KEY = 'openhuman:walkthrough_pending';

beforeEach(() => {
  localStorage.clear();
});

afterEach(() => {
  localStorage.clear();
  vi.resetModules();
});

// ── Helper state tests ────────────────────────────────────────────────────

describe('isWalkthroughPending', () => {
  it('returns false when nothing is set', () => {
    expect(isWalkthroughPending()).toBe(false);
  });

  it('returns true when pending flag is set and completed flag is not', () => {
    localStorage.setItem(WALKTHROUGH_PENDING_KEY, 'true');
    expect(isWalkthroughPending()).toBe(true);
  });

  it('returns false when both pending and completed are set', () => {
    localStorage.setItem(WALKTHROUGH_PENDING_KEY, 'true');
    localStorage.setItem(WALKTHROUGH_KEY, 'true');
    expect(isWalkthroughPending()).toBe(false);
  });

  it('returns false when only completed flag is set', () => {
    localStorage.setItem(WALKTHROUGH_KEY, 'true');
    expect(isWalkthroughPending()).toBe(false);
  });
});

describe('setWalkthroughPending', () => {
  it('sets the pending flag in localStorage', () => {
    setWalkthroughPending();
    expect(localStorage.getItem(WALKTHROUGH_PENDING_KEY)).toBe('true');
  });
});

describe('markWalkthroughComplete', () => {
  it('sets the completed flag and removes the pending flag', () => {
    localStorage.setItem(WALKTHROUGH_PENDING_KEY, 'true');
    markWalkthroughComplete();
    expect(localStorage.getItem(WALKTHROUGH_KEY)).toBe('true');
    expect(localStorage.getItem(WALKTHROUGH_PENDING_KEY)).toBeNull();
  });
});

// ── AppWalkthrough component tests ────────────────────────────────────────

describe('AppWalkthrough component', () => {
  it('renders Joyride when walkthrough is pending', async () => {
    setWalkthroughPending();

    const { default: AppWalkthrough } = await import('../AppWalkthrough');
    render(<AppWalkthrough />);

    expect(screen.getByTestId('joyride-mock')).toBeInTheDocument();
    expect(screen.getByTestId('joyride-mock').getAttribute('data-run')).toBe('true');
  });

  it('renders nothing when walkthrough is not pending', async () => {
    // No pending flag set

    const { default: AppWalkthrough } = await import('../AppWalkthrough');
    const { container } = render(<AppWalkthrough />);

    expect(container.firstChild).toBeNull();
  });

  it('renders nothing when walkthrough is already completed', async () => {
    // Set pending but also completed — should not render
    localStorage.setItem(WALKTHROUGH_PENDING_KEY, 'true');
    localStorage.setItem(WALKTHROUGH_KEY, 'true');

    const { default: AppWalkthrough } = await import('../AppWalkthrough');
    const { container } = render(<AppWalkthrough />);

    expect(container.firstChild).toBeNull();
  });
});

// ── walkthroughSteps tests ────────────────────────────────────────────────

describe('WALKTHROUGH_STEPS', () => {
  it('has 6 steps', () => {
    expect(WALKTHROUGH_STEPS).toHaveLength(6);
  });

  it('first step targets home-card and disables beacon', () => {
    const first = WALKTHROUGH_STEPS[0];
    expect(first.target).toBe('[data-walkthrough="home-card"]');
    expect(first.skipBeacon).toBe(true);
  });

  it('last step targets tab-settings', () => {
    const last = WALKTHROUGH_STEPS[WALKTHROUGH_STEPS.length - 1];
    expect(last.target).toBe('[data-walkthrough="tab-settings"]');
  });

  it('all steps have a title and content', () => {
    for (const step of WALKTHROUGH_STEPS) {
      expect(step.title).toBeTruthy();
      expect(step.content).toBeTruthy();
    }
  });
});
