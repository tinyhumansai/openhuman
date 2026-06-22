import { useCallback, useMemo, useState } from 'react';
import { Outlet, useNavigate } from 'react-router-dom';

import { setWalkthroughPending } from '../../components/walkthrough/AppWalkthrough';
import WalkthroughContainer from '../../components/walkthrough/WalkthroughContainer';
import { useCoreState } from '../../providers/CoreStateProvider';
import { trackEvent } from '../../services/analytics';
import { getDefaultEnabledTools, getEnabledRustToolNames } from '../../utils/toolDefinitions';
import BetaBanner from './components/BetaBanner';
import {
  OnboardingContext,
  type OnboardingDraft,
  type WalkthroughPhase,
  type WalkthroughState,
} from './OnboardingContext';

/** Ordered list of walkthrough phases in the narrative arc. */
const WALKTHROUGH_PHASES: WalkthroughPhase[] = ['welcome', 'connect', 'automate', 'review', 'done'];

/** Default action-card step for the welcome phase. */
const WELCOME_STEPS = [{ key: 'start', completed: false }];

/** Default action-card steps for the connect phase. */
const CONNECT_STEPS = [
  { key: 'gmail', completed: false },
  { key: 'slack', completed: false },
  { key: 'whatsapp', completed: false },
  { key: 'telegram', completed: false },
  { key: 'discord', completed: false },
];

/** Default action-card steps for the automate phase. */
const AUTOMATE_STEPS = [
  { key: 'briefings', completed: false },
  { key: 'notifications', completed: false },
  { key: 'scheduling', completed: false },
  { key: 'summaries', completed: false },
];

function createInitialWalkthrough(): WalkthroughState {
  return { phase: 'welcome', steps: WELCOME_STEPS, completed: false, skipped: false };
}

/**
 * Full-page chrome for the onboarding flow. Hosts the shared draft + the
 * completion side-effects (persist `onboarding_completed`, notify backend,
 * navigate to /home). Individual steps render through `<Outlet />`.
 *
 * Also manages the integrated walkthrough narrative state, advancing through
 * preview phases without claiming setup work happened outside real setup flows.
 */
const OnboardingLayout = () => {
  const navigate = useNavigate();
  const { setOnboardingCompletedFlag, setOnboardingTasks, snapshot } = useCoreState();
  const [draft, setDraftState] = useState<OnboardingDraft>({
    connectedSources: [],
    walkthrough: createInitialWalkthrough(),
  });

  const setDraft = useCallback(
    (updater: (prev: OnboardingDraft) => OnboardingDraft) => setDraftState(updater),
    []
  );

  const advanceWalkthrough = useCallback((stepKey?: string): void => {
    setDraftState(prev => {
      const wt = prev.walkthrough ?? createInitialWalkthrough();

      // If a specific step key is given, mark it completed.
      let steps = wt.steps;
      if (stepKey) {
        steps = steps.map(s => (s.key === stepKey ? { ...s, completed: true } : s));
      }

      // Phase-level continue advances informational walkthrough cards without
      // claiming that integrations or automations were actually configured.
      const shouldAdvance = stepKey ? steps.every(s => s.completed) : true;

      // Advance to next phase if all steps are completed.
      let nextPhase: WalkthroughPhase = wt.phase;
      if (shouldAdvance) {
        const currentIdx = WALKTHROUGH_PHASES.indexOf(wt.phase);
        if (currentIdx < WALKTHROUGH_PHASES.length - 1) {
          nextPhase = WALKTHROUGH_PHASES[currentIdx + 1];
        }
      }

      // Load steps for the next phase.
      let nextSteps = steps;
      if (nextPhase !== wt.phase) {
        if (nextPhase === 'connect') {
          nextSteps = CONNECT_STEPS;
        } else if (nextPhase === 'automate') {
          nextSteps = AUTOMATE_STEPS;
        } else if (nextPhase === 'review') {
          nextSteps = steps;
        } else if (nextPhase === 'done') {
          nextSteps = [];
        }
      }

      return {
        ...prev,
        walkthrough: {
          phase: nextPhase,
          steps: nextSteps,
          completed: nextPhase === 'done',
          skipped: wt.skipped,
        },
      };
    });
  }, []);

  const skipWalkthrough = useCallback(() => {
    setDraftState(prev => ({
      ...prev,
      walkthrough: { phase: 'review', steps: [], completed: false, skipped: true },
    }));
  }, []);

  const completeAndExit = useCallback(async () => {
    console.debug('[onboarding:layout] completeAndExit', {
      connectedSources: draft.connectedSources,
    });

    try {
      // Preserve a tool preference the user already customized (e.g. via
      // Settings → Tools or an earlier onboarding run) rather than resetting
      // to catalog defaults on every completion. Re-applying defaults here
      // could silently narrow an existing selection. Only seed defaults when
      // no preference has been persisted yet. The Rust-side filter
      // (`filter_tools_by_user_preference`) is the authoritative guard against
      // stale snapshots stripping newer tools (issue #3096); this is
      // defense-in-depth on the write path.
      const existingEnabledTools = snapshot.localState.onboardingTasks?.enabledTools;
      const enabledTools =
        existingEnabledTools && existingEnabledTools.length > 0
          ? existingEnabledTools
          : getEnabledRustToolNames(getDefaultEnabledTools());

      await setOnboardingTasks({
        accessibilityPermissionGranted:
          snapshot.localState.onboardingTasks?.accessibilityPermissionGranted ?? false,
        localModelConsentGiven: false,
        localModelDownloadStarted: false,
        enabledTools,
        connectedSources: draft.connectedSources,
        updatedAtMs: Date.now(),
      });
    } catch (e) {
      console.warn('[onboarding] Failed to persist onboarding tasks; continuing completion', e);
    }

    try {
      await setOnboardingCompletedFlag(true);
    } catch (e) {
      console.error('[onboarding] Failed to persist onboarding_completed', e);
      throw e;
    }

    // Fire onboarding_complete analytics event before navigation.
    trackEvent('onboarding_complete');

    // Flag the Joyride walkthrough as pending so it auto-starts on /home.
    // Best-effort: localStorage failures must not block navigation.
    try {
      setWalkthroughPending();
      console.debug('[onboarding:layout] walkthrough pending flag set — navigating to /home');
    } catch (e) {
      console.warn('[onboarding:layout] could not set walkthrough pending flag; continuing', e);
    }

    navigate('/home', { replace: true });
  }, [draft.connectedSources, navigate, setOnboardingCompletedFlag, setOnboardingTasks, snapshot]);

  const value = useMemo(
    () => ({ draft, setDraft, completeAndExit, advanceWalkthrough, skipWalkthrough }),
    [draft, setDraft, completeAndExit, advanceWalkthrough, skipWalkthrough]
  );

  return (
    <OnboardingContext.Provider value={value}>
      <div
        data-testid="onboarding-layout"
        className="min-h-full relative flex items-center justify-center py-10">
        <div className="relative z-10 w-full max-w-2xl mx-4">
          <BetaBanner />
          <Outlet />
          <div className="mt-4 rounded-2xl bg-white dark:bg-neutral-900 p-6 shadow-soft">
            <WalkthroughContainer />
          </div>
        </div>
      </div>
    </OnboardingContext.Provider>
  );
};

export default OnboardingLayout;
