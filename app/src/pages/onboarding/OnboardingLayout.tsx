import { invoke, isTauri } from '@tauri-apps/api/core';
import { useCallback, useMemo, useState } from 'react';
import { Outlet, useNavigate } from 'react-router-dom';

import { useCoreState } from '../../providers/CoreStateProvider';
import { userApi } from '../../services/api/userApi';
import { useAppDispatch } from '../../store/hooks';
import { createNewThread, setSelectedThread, setWelcomeThreadId } from '../../store/threadSlice';
import { getDefaultEnabledTools } from '../../utils/toolDefinitions';
import BetaBanner from './components/BetaBanner';
import { OnboardingContext, type OnboardingDraft } from './OnboardingContext';

/**
 * Full-page chrome for the onboarding flow. Hosts the shared draft + the
 * completion side-effects (persist `onboarding_completed`, notify backend,
 * navigate to /home). Individual steps render through `<Outlet />`.
 */
const OnboardingLayout = () => {
  const navigate = useNavigate();
  const dispatch = useAppDispatch();
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

    await setOnboardingTasks({
      accessibilityPermissionGranted:
        snapshot.localState.onboardingTasks?.accessibilityPermissionGranted ?? false,
      localModelConsentGiven: false,
      localModelDownloadStarted: false,
      enabledTools: getDefaultEnabledTools(),
      connectedSources: draft.connectedSources,
      updatedAtMs: Date.now(),
    });

    try {
      await userApi.onboardingComplete();
    } catch {
      console.warn('[onboarding] Failed to notify backend of onboarding completion');
    }

    try {
      await setOnboardingCompletedFlag(true);
    } catch (e) {
      console.error('[onboarding] Failed to persist onboarding_completed', e);
      throw e;
    }

    // Open a fresh chat thread for the welcome conversation so the
    // proactive messages don't pile onto whatever thread the user had
    // open before onboarding. The proactive subscriber resolves the
    // `proactive:welcome` thread_id to whichever thread is currently
    // selected, so we need a new selected thread *before* firing the
    // agent — otherwise the welcome message lands in the user's
    // pre-onboarding thread.
    //
    // If the thread create fails we deliberately skip the spawn, since
    // it would publish the proactive message to whichever thread happens
    // to be selected (or worse, fail to land anywhere). The user can
    // trigger the welcome again by sending their first message in chat
    // (which routes to welcome while `chat_onboarding_completed` is
    // still false).
    let welcomeReady = false;
    try {
      const newThread = await dispatch(createNewThread()).unwrap();
      dispatch(setSelectedThread(newThread.id));
      // Track this thread so the post-onboarding watcher can delete it
      // once `chat_onboarding_completed` flips. The welcome conversation
      // is transient — we don't keep it in the user's thread list.
      dispatch(setWelcomeThreadId(newThread.id));
      welcomeReady = true;
    } catch (e) {
      console.warn(
        '[onboarding] failed to create welcome thread; skipping spawn_welcome_agent',
        e
      );
    }

    // Trigger the proactive welcome agent now that the welcome thread is
    // ready. Core-side spawning was removed in favor of renderer-owned
    // timing so we can fire after `/home` is the active surface and the
    // chat UI is ready to receive the messages.
    if (welcomeReady && isTauri()) {
      try {
        await invoke('spawn_welcome_agent');
      } catch (e) {
        console.warn('[onboarding] failed to spawn welcome agent', e);
      }
    }

    navigate('/home', { replace: true });
  }, [
    draft.connectedSources,
    dispatch,
    navigate,
    setOnboardingCompletedFlag,
    setOnboardingTasks,
    snapshot,
  ]);

  const value = useMemo(
    () => ({ draft, setDraft, completeAndExit }),
    [draft, setDraft, completeAndExit]
  );

  return (
    <OnboardingContext.Provider value={value}>
      <div
        data-testid="onboarding-layout"
        className="min-h-full relative flex items-center justify-center py-10">
        <div className="relative z-10 w-full max-w-lg mx-4">
          <BetaBanner />
          <Outlet />
        </div>
      </div>
    </OnboardingContext.Provider>
  );
};

export default OnboardingLayout;
