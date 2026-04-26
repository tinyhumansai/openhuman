import * as Sentry from '@sentry/react';
import { useEffect } from 'react';
import { Provider } from 'react-redux';
import { HashRouter as Router, useLocation, useNavigate } from 'react-router-dom';
import { PersistGate } from 'redux-persist/integration/react';

import AppRoutes from './AppRoutes';
import BottomTabBar from './components/BottomTabBar';
import CommandProvider from './components/commands/CommandProvider';
import ServiceBlockingGate from './components/daemon/ServiceBlockingGate';
import DictationHotkeyManager from './components/DictationHotkeyManager';
import ErrorFallbackScreen from './components/ErrorFallbackScreen';
import LocalAIDownloadSnackbar from './components/LocalAIDownloadSnackbar';
import MeshGradient from './components/MeshGradient';
import OpenhumanLinkModal from './components/OpenhumanLinkModal';
import RouteLoadingScreen from './components/RouteLoadingScreen';
import GlobalUpsellBanner from './components/upsell/GlobalUpsellBanner';
import { isWelcomeLocked } from './lib/coreState/store';
import { startNativeNotificationsService } from './lib/nativeNotifications';
import { startWebviewNotificationsService } from './lib/webviewNotifications';
import ChatRuntimeProvider from './providers/ChatRuntimeProvider';
import CoreStateProvider, { useCoreState } from './providers/CoreStateProvider';
import SocketProvider from './providers/SocketProvider';
import { tagErrorSource } from './services/errorReportQueue';
import { startWebviewAccountService } from './services/webviewAccountService';
import { persistor, store } from './store';
import { useAppDispatch, useAppSelector } from './store/hooks';
import { clearSelectedThread, deleteThread, setWelcomeThreadId } from './store/threadSlice';
import { isAccountsFullscreen } from './utils/accountsFullscreen';
import { DEV_FORCE_ONBOARDING } from './utils/config';

// Attach the `webview:event` listener at app boot so background recipe
// events (Google Meet captions → transcript flush, WhatsApp ingest, …)
// are handled even when the user hasn't navigated to /accounts yet.
// Idempotent — the service uses a `started` singleton guard.
startWebviewAccountService();
startWebviewNotificationsService();
startNativeNotificationsService();

function App() {
  return (
    <Sentry.ErrorBoundary
      fallback={({ error, componentStack, resetError }) => (
        <ErrorFallbackScreen error={error} componentStack={componentStack} onReset={resetError} />
      )}
      onError={(_error, componentStack, eventId) => {
        tagErrorSource(eventId, 'react', componentStack);
      }}>
      <Provider store={store}>
        <PersistGate loading={<RouteLoadingScreen />} persistor={persistor}>
          <CoreStateProvider>
            <SocketProvider>
              <ChatRuntimeProvider>
                <Router>
                  <CommandProvider>
                    <ServiceBlockingGate>
                      <AppShell />
                      <DictationHotkeyManager />
                      <LocalAIDownloadSnackbar />
                    </ServiceBlockingGate>
                  </CommandProvider>
                </Router>
              </ChatRuntimeProvider>
            </SocketProvider>
          </CoreStateProvider>
        </PersistGate>
      </Provider>
    </Sentry.ErrorBoundary>
  );
}

/** Inner shell — lives inside the Router so it can use useLocation. */
function AppShell() {
  const location = useLocation();
  const navigate = useNavigate();
  const { snapshot, isBootstrapping } = useCoreState();
  const activeAccountId = useAppSelector(state => state.accounts.activeAccountId);
  // On /accounts, only the agent view keeps the tab bar + its reserved
  // bottom padding. Any other selected "app" (e.g. WhatsApp) takes the
  // full viewport so the embedded webview goes edge-to-edge.
  const fullscreen = isAccountsFullscreen(location.pathname, activeAccountId);
  const welcomeLocked = isWelcomeLocked(snapshot);
  const onOnboardingRoute = location.pathname.startsWith('/onboarding');
  const onboardingPending =
    !!snapshot.sessionToken && (DEV_FORCE_ONBOARDING || !snapshot.onboardingCompleted);

  // Onboarding gate: while `onboarding_completed=false`, force any non-
  // onboarding route back to `/onboarding`. Once completed, bounce the
  // user off `/onboarding` so they don't get stuck on the stepper.
  useEffect(() => {
    if (isBootstrapping || !snapshot.sessionToken) return;
    if (onboardingPending && !onOnboardingRoute) {
      console.debug(
        `[onboarding-gate] redirecting ${location.pathname} -> /onboarding (onboarding incomplete)`
      );
      navigate('/onboarding', { replace: true });
    } else if (!onboardingPending && onOnboardingRoute) {
      console.debug(
        `[onboarding-gate] redirecting ${location.pathname} -> /home (onboarding complete)`
      );
      navigate('/home', { replace: true });
    }
  }, [
    isBootstrapping,
    snapshot.sessionToken,
    onboardingPending,
    onOnboardingRoute,
    location.pathname,
    navigate,
  ]);

  // After the welcome agent calls `complete_onboarding` and
  // `chat_onboarding_completed` flips false→true, discard the transient
  // welcome thread we created in `OnboardingLayout`. The next user
  // message will route to the orchestrator and create its own thread.
  const dispatch = useAppDispatch();
  const welcomeThreadId = useAppSelector(state => state.thread.welcomeThreadId);
  const chatOnboardingCompleted = snapshot.chatOnboardingCompleted;
  useEffect(() => {
    if (!chatOnboardingCompleted || !welcomeThreadId) return;
    let cancelled = false;
    console.debug(
      `[welcome-cleanup] chat_onboarding_completed=true — deleting welcome thread ${welcomeThreadId}`
    );
    // Await the delete before dropping the local id so a backend failure
    // leaves `welcomeThreadId` set for retry on the next render. Without
    // the await, a 500 from `threads.delete` would leave a stale row in
    // the user's thread list while the renderer thinks it's gone.
    (async () => {
      try {
        await dispatch(deleteThread(welcomeThreadId)).unwrap();
        if (cancelled) return;
        dispatch(clearSelectedThread());
        dispatch(setWelcomeThreadId(null));
      } catch (err) {
        console.warn('[welcome-cleanup] deleteThread failed; will retry on next render', err);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [chatOnboardingCompleted, welcomeThreadId, dispatch]);

  // Welcome lockdown (#883) — force any route other than `/chat` back to
  // `/chat` while the welcome-agent conversation is still in progress.
  // Skipped while onboarding is still pending (the onboarding gate above
  // owns the route during that phase).
  useEffect(() => {
    if (!welcomeLocked || isBootstrapping) return;
    if (onboardingPending) return;
    if (location.pathname === '/chat') return;
    console.debug(
      `[welcome-lock] redirecting ${location.pathname} -> /chat (chat onboarding incomplete)`
    );
    navigate('/chat', { replace: true });
  }, [welcomeLocked, isBootstrapping, onboardingPending, location.pathname, navigate]);

  return (
    <div className="relative h-screen flex flex-col overflow-hidden">
      <MeshGradient />
      <div className="app-dotted-canvas relative z-10 flex-1 flex flex-col overflow-hidden">
        <div
          className={`flex-1 overflow-y-auto ${
            fullscreen || welcomeLocked || onOnboardingRoute ? '' : 'pb-16'
          }`}>
          <GlobalUpsellBanner />
          <AppRoutes />
        </div>
        {!onOnboardingRoute && <BottomTabBar />}
      </div>
      <OpenhumanLinkModal />
    </div>
  );
}

export default App;
