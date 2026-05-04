/**
 * Tests for the Joyride walkthrough components introduced in #1123.
 *
 * Verifies:
 *  - isWalkthroughPending / setWalkthroughPending / markWalkthroughComplete helpers
 *  - AppWalkthrough renders only when pending
 *  - AppWalkthrough does not render when already completed
 *  - Completing/skipping the tour calls markWalkthroughComplete (localStorage set)
 *  - Step count matches WALKTHROUGH_STEPS
 *  - WalkthroughTooltip renders step title, content, and navigation buttons
 */
import { act, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  isWalkthroughPending,
  markWalkthroughComplete,
  setWalkthroughPending,
} from '../AppWalkthrough';
import { WALKTHROUGH_STEPS } from '../walkthroughSteps';
// ── WalkthroughTooltip rendering tests ───────────────────────────────────

import WalkthroughTooltip from '../WalkthroughTooltip';

// ── Mock react-joyride so tests don't need a real DOM with
//    positioned elements for each step target. ─────────────────────────────
//    The mock captures the `onEvent` callback so individual tests can
//    simulate tour events (TOUR_END with FINISHED / SKIPPED status).

type JoyrideMockProps = {
  run: boolean;
  onEvent?: (data: { type: string; status: string; index: number }) => void;
};

let capturedOnEvent: JoyrideMockProps['onEvent'] | undefined;

vi.mock('react-joyride', () => ({
  Joyride: ({ run, onEvent }: JoyrideMockProps) => {
    capturedOnEvent = onEvent;
    return <div data-testid="joyride-mock" data-run={String(run)} />;
  },
  EVENTS: { TOUR_END: 'tour:end' },
  STATUS: { FINISHED: 'finished', SKIPPED: 'skipped' },
}));

// ── localStorage helpers ───────────────────────────────────────────────────

const WALKTHROUGH_KEY = 'openhuman:walkthrough_completed';
const WALKTHROUGH_PENDING_KEY = 'openhuman:walkthrough_pending';

beforeEach(() => {
  localStorage.clear();
  capturedOnEvent = undefined;
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

  it('swallows error when localStorage.setItem throws (SecurityError / quota)', () => {
    // Temporarily replace localStorage with a broken implementation to trigger
    // the catch block at line 44 in setWalkthroughPending.
    const realStorage = globalThis.localStorage;
    Object.defineProperty(globalThis, 'localStorage', {
      value: {
        ...realStorage,
        setItem() {
          throw new DOMException('QuotaExceededError', 'QuotaExceededError');
        },
      },
      configurable: true,
      writable: true,
    });

    try {
      // Should not throw — the error is swallowed inside setWalkthroughPending
      expect(() => setWalkthroughPending()).not.toThrow();
    } finally {
      Object.defineProperty(globalThis, 'localStorage', {
        value: realStorage,
        configurable: true,
        writable: true,
      });
    }
  });
});

describe('markWalkthroughComplete', () => {
  it('sets the completed flag and removes the pending flag', () => {
    localStorage.setItem(WALKTHROUGH_PENDING_KEY, 'true');
    markWalkthroughComplete();
    expect(localStorage.getItem(WALKTHROUGH_KEY)).toBe('true');
    expect(localStorage.getItem(WALKTHROUGH_PENDING_KEY)).toBeNull();
  });

  it('swallows error when localStorage.setItem throws (SecurityError / quota)', () => {
    // Temporarily replace localStorage with a broken implementation to trigger
    // the catch block at line 61 in markWalkthroughComplete.
    const realStorage = globalThis.localStorage;
    Object.defineProperty(globalThis, 'localStorage', {
      value: {
        ...realStorage,
        setItem() {
          throw new DOMException('QuotaExceededError', 'QuotaExceededError');
        },
      },
      configurable: true,
      writable: true,
    });

    try {
      // Should not throw — the error is swallowed inside markWalkthroughComplete
      expect(() => markWalkthroughComplete()).not.toThrow();
    } finally {
      Object.defineProperty(globalThis, 'localStorage', {
        value: realStorage,
        configurable: true,
        writable: true,
      });
    }
  });
});

describe('isWalkthroughPending — localStorage unavailable', () => {
  it('returns false and swallows error when localStorage.getItem throws', () => {
    // Temporarily replace localStorage with a broken implementation to trigger
    // the catch block at lines 26-27 in isWalkthroughPending.
    const realStorage = globalThis.localStorage;
    Object.defineProperty(globalThis, 'localStorage', {
      value: {
        ...realStorage,
        getItem() {
          throw new DOMException('SecurityError', 'SecurityError');
        },
      },
      configurable: true,
      writable: true,
    });

    try {
      // Should return false (the catch branch) and not throw
      expect(isWalkthroughPending()).toBe(false);
    } finally {
      Object.defineProperty(globalThis, 'localStorage', {
        value: realStorage,
        configurable: true,
        writable: true,
      });
    }
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

  it('calls markWalkthroughComplete and stops running when tour finishes (FINISHED)', async () => {
    setWalkthroughPending();

    const { default: AppWalkthrough } = await import('../AppWalkthrough');
    render(<AppWalkthrough />);

    // Joyride should be running initially
    expect(screen.getByTestId('joyride-mock').getAttribute('data-run')).toBe('true');

    // Simulate TOUR_END with FINISHED status
    await act(async () => {
      capturedOnEvent?.({ type: 'tour:end', status: 'finished', index: 5 });
    });

    // Walkthrough should be marked complete in localStorage
    expect(localStorage.getItem(WALKTHROUGH_KEY)).toBe('true');
    expect(localStorage.getItem(WALKTHROUGH_PENDING_KEY)).toBeNull();
  });

  it('calls markWalkthroughComplete and stops running when tour is skipped (SKIPPED)', async () => {
    setWalkthroughPending();

    const { default: AppWalkthrough } = await import('../AppWalkthrough');
    render(<AppWalkthrough />);

    expect(screen.getByTestId('joyride-mock').getAttribute('data-run')).toBe('true');

    // Simulate TOUR_END with SKIPPED status
    await act(async () => {
      capturedOnEvent?.({ type: 'tour:end', status: 'skipped', index: 1 });
    });

    expect(localStorage.getItem(WALKTHROUGH_KEY)).toBe('true');
    expect(localStorage.getItem(WALKTHROUGH_PENDING_KEY)).toBeNull();
  });

  it('does not call markWalkthroughComplete for non-TOUR_END events', async () => {
    setWalkthroughPending();

    const { default: AppWalkthrough } = await import('../AppWalkthrough');
    render(<AppWalkthrough />);

    // Simulate a step:after event (not tour:end)
    await act(async () => {
      capturedOnEvent?.({ type: 'step:after', status: 'running', index: 0 });
    });

    // Should NOT have marked complete
    expect(localStorage.getItem(WALKTHROUGH_KEY)).toBeNull();
    // Still running
    expect(screen.getByTestId('joyride-mock')).toBeInTheDocument();
  });
});

/** Build the minimal props required by WalkthroughTooltip without fighting the full TooltipRenderProps type. */
function makeTooltipProps(
  overrides: {
    index?: number;
    size?: number;
    isLastStep?: boolean;
    continuous?: boolean;
    title?: string;
    content?: string;
  } = {}
) {
  const {
    index = 0,
    size = 3,
    isLastStep = false,
    continuous = true,
    title = 'Step title',
    content = 'Step content',
  } = overrides;
  // Cast to unknown then to the component's expected props to avoid fighting
  // the exhaustive TooltipRenderProps type in test code.
  return {
    continuous,
    index,
    size,
    isLastStep,
    step: { title, content, target: 'body' },
    backProps: {
      'aria-label': 'Back',
      onClick: vi.fn(),
      role: 'button',
      title: 'Back',
      'data-action': 'back',
    },
    primaryProps: {
      'aria-label': 'Next',
      onClick: vi.fn(),
      role: 'button',
      title: 'Next',
      'data-action': 'primary',
    },
    skipProps: {
      'aria-label': 'Skip',
      onClick: vi.fn(),
      role: 'button',
      title: 'Skip',
      'data-action': 'skip',
    },
    tooltipProps: { role: 'tooltip' },
    closeProps: {
      'aria-label': 'Close',
      onClick: vi.fn(),
      role: 'button',
      title: 'Close',
      'data-action': 'close',
    },
  } as unknown as Parameters<typeof WalkthroughTooltip>[0];
}

describe('WalkthroughTooltip', () => {
  it('renders step title and content', () => {
    render(<WalkthroughTooltip {...makeTooltipProps()} />);

    expect(screen.getByText('Step title')).toBeInTheDocument();
    expect(screen.getByText('Step content')).toBeInTheDocument();
  });

  it('renders step counter showing current step of total', () => {
    render(<WalkthroughTooltip {...makeTooltipProps({ index: 1, size: 6 })} />);

    expect(screen.getByText('2 of 6')).toBeInTheDocument();
  });

  it('shows Skip button when not on last step', () => {
    render(<WalkthroughTooltip {...makeTooltipProps({ isLastStep: false })} />);

    expect(screen.getByText('Skip tour')).toBeInTheDocument();
  });

  it('hides Skip button on the last step', () => {
    render(<WalkthroughTooltip {...makeTooltipProps({ isLastStep: true })} />);

    expect(screen.queryByText('Skip tour')).toBeNull();
  });

  it('shows Finish on the last step', () => {
    render(<WalkthroughTooltip {...makeTooltipProps({ isLastStep: true })} />);

    expect(screen.getByText("Let's go!")).toBeInTheDocument();
  });

  it('shows Next on non-last steps', () => {
    render(<WalkthroughTooltip {...makeTooltipProps({ isLastStep: false })} />);

    expect(screen.getByText('Next →')).toBeInTheDocument();
  });

  it('hides Back button on the first step (index 0)', () => {
    render(<WalkthroughTooltip {...makeTooltipProps({ index: 0 })} />);

    expect(screen.queryByText('Back')).toBeNull();
  });

  it('shows Back button after the first step', () => {
    render(<WalkthroughTooltip {...makeTooltipProps({ index: 1 })} />);

    expect(screen.getByText('Back')).toBeInTheDocument();
  });

  it('renders progress bar', () => {
    const { container } = render(<WalkthroughTooltip {...makeTooltipProps({ index: 2, size: 6 })} />);

    // Gradient progress bar fills based on step progress
    const bar = container.querySelector('div.bg-gradient-to-r');
    expect(bar).not.toBeNull();
    expect(bar?.getAttribute('style')).toContain('width: 50%');
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
