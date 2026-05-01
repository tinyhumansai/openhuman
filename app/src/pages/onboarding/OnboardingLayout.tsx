import { useCallback, useMemo, useState } from 'react';
import { Outlet, useNavigate } from 'react-router-dom';

import { useCoreState } from '../../providers/CoreStateProvider';
import { userApi } from '../../services/api/userApi';
import { chatSend } from '../../services/chatService';
import { useAppDispatch } from '../../store/hooks';
import { createNewThread, setSelectedThread, setWelcomeThreadId } from '../../store/threadSlice';
import { getDefaultEnabledTools } from '../../utils/toolDefinitions';
import BetaBanner from './components/BetaBanner';
import { OnboardingContext, type OnboardingDraft } from './OnboardingContext';

/**
 * Synthetic "user" message handed to the welcome agent on the first turn
 * after onboarding completes. Routed through the normal `chat_send`
 * dispatch path (instead of an out-of-band `agent.run_single` proactive
 * bypass) so the welcome agent's reply lands in the thread's per-sender
 * history cache. Subsequent real user messages then see the full prior
 * turn and continue the conversation rather than starting fresh.
 *
 * The welcome agent's `prompt.md` matches on this exact string and
 * applies its opening voice. Don't change without updating the
 * prompt's "Proactive opening" section.
 *
 * The trigger is **not** persisted as a user-side bubble (we skip
 * `addMessageLocal`), so the user only sees the agent's reply.
 */
const WELCOME_TRIGGER_MESSAGE =
  'the user just finished the desktop onboarding wizard. welcome the user. say something interesting from the profile information above';

/**
 * Model id used for the welcome trigger send. Mirrors the constant in
 * `pages/Conversations.tsx` (`CHAT_MODEL_ID`); duplicated here to avoid
 * pulling the entire conversations module into onboarding.
 */
const WELCOME_TRIGGER_MODEL = 'reasoning-v1';

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
    // welcome opener doesn't pile onto whatever thread the user had
    // open before onboarding. We then fire the welcome trigger through
    // the normal `chat_send` dispatch path (NOT an out-of-band proactive
    // spawn) so the agent's reply lands in the thread's per-sender
    // history cache and subsequent real user messages can continue the
    // conversation with full prior context.
    //
    // If the thread create fails we skip the trigger; the user can fire
    // the welcome again by sending their first message in chat (which
    // routes to welcome while `chat_onboarding_completed` is still
    // false).
    let welcomeThread: { id: string } | null = null;
    try {
      const newThread = await dispatch(createNewThread()).unwrap();
      dispatch(setSelectedThread(newThread.id));
      // Track this thread so the post-onboarding watcher can delete it
      // once `chat_onboarding_completed` flips. The welcome conversation
      // is transient — we don't keep it in the user's thread list.
      dispatch(setWelcomeThreadId(newThread.id));
      welcomeThread = { id: newThread.id };
    } catch (e) {
      console.warn('[onboarding] failed to create welcome thread; skipping welcome trigger', e);
    }

    if (welcomeThread) {
      try {
        // NB: deliberately *not* calling `addMessageLocal` for the
        // trigger so it doesn't render as a user-side bubble. The agent
        // response comes back via socket → `addInferenceResponse` and
        // is the first thing the user sees in the welcome thread.
        await chatSend({
          threadId: welcomeThread.id,
          message: WELCOME_TRIGGER_MESSAGE,
          model: WELCOME_TRIGGER_MODEL,
        });
      } catch (e) {
        console.warn('[onboarding] failed to fire welcome trigger', e);
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
