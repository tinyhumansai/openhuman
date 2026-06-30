import { useCallback, useMemo, useState } from 'react';
import { Outlet, useNavigate } from 'react-router-dom';

import { setWalkthroughPending } from '../../components/walkthrough/AppWalkthrough';
import { useCoreState } from '../../providers/CoreStateProvider';
import { trackEvent } from '../../services/analytics';
import { getDefaultEnabledTools, getEnabledRustToolNames } from '../../utils/toolDefinitions';
import BetaBanner from './components/BetaBanner';
import { OnboardingContext, type OnboardingDraft } from './OnboardingContext';

/**
 * Full-page chrome for the onboarding flow. Hosts the shared draft + the
 * completion side-effects (persist `onboarding_completed`, notify backend,
 * navigate to /chat). Individual steps render through `<Outlet />`.
 */
const OnboardingLayout = () => {
  const navigate = useNavigate();
  const { setOnboardingCompletedFlag, setOnboardingTasks, snapshot } = useCoreState();
  const [draft, setDraftState] = useState<OnboardingDraft>({ connectedSources: [] });

  const setDraft = useCallback(
    (updater: (prev: OnboardingDraft) => OnboardingDraft) => setDraftState(updater),
    []
  );

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

    // Flag the Joyride walkthrough as pending so it auto-starts on the chat landing surface.
    // Best-effort: localStorage failures must not block navigation.
    try {
      setWalkthroughPending();
      console.debug('[onboarding:layout] walkthrough pending flag set — navigating to /chat');
    } catch (e) {
      console.warn('[onboarding:layout] could not set walkthrough pending flag; continuing', e);
    }

    navigate('/chat', { replace: true });
  }, [draft.connectedSources, navigate, setOnboardingCompletedFlag, setOnboardingTasks, snapshot]);

  const value = useMemo(
    () => ({ draft, setDraft, completeAndExit }),
    [draft, setDraft, completeAndExit]
  );

  return (
    <OnboardingContext.Provider value={value}>
      <div
        data-testid="onboarding-layout"
        className="min-h-full relative flex items-center justify-center py-10">
        <div className="relative z-10 w-full max-w-2xl mx-4">
          <BetaBanner />
          <Outlet />
        </div>
      </div>
    </OnboardingContext.Provider>
  );
};

export default OnboardingLayout;
