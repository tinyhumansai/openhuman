import * as Sentry from '@sentry/react';
import { useEffect } from 'react';
import { Provider } from 'react-redux';
import { HashRouter as Router, useLocation, useNavigate } from 'react-router-dom';
import { PersistGate } from 'redux-persist/integration/react';

import AppRoutes from './AppRoutes';
import AppBackground from './components/AppBackground';
import AppUpdatePrompt from './components/AppUpdatePrompt';
import BootCheckGate from './components/BootCheckGate/BootCheckGate';
import BottomTabBar from './components/BottomTabBar';
import CommandProvider from './components/commands/CommandProvider';
import ServiceBlockingGate from './components/daemon/ServiceBlockingGate';
import DictationHotkeyManager from './components/DictationHotkeyManager';
import ErrorFallbackScreen from './components/ErrorFallbackScreen';
import LocalAIDownloadSnackbar from './components/LocalAIDownloadSnackbar';
import OpenhumanLinkModal from './components/OpenhumanLinkModal';
import PersistRehydrationScreen from './components/PersistRehydrationScreen';
import GlobalUpsellBanner from './components/upsell/GlobalUpsellBanner';
import AppWalkthrough from './components/walkthrough/AppWalkthrough';
import { MascotFrameProducer } from './features/meet/MascotFrameProducer';
import { I18nProvider } from './lib/i18n/I18nContext';
import {
  startNativeNotificationsService,
  stopNativeNotificationsService,
} from './lib/nativeNotifications';
import { getIsMobile } from './lib/platform';
import {
  startWebviewNotificationsService,
  stopWebviewNotificationsService,
} from './lib/webviewNotifications';
import ChatRuntimeProvider from './providers/ChatRuntimeProvider';
import CoreStateProvider, { useCoreState } from './providers/CoreStateProvider';
import SocketProvider from './providers/SocketProvider';
import ThemeProvider from './providers/ThemeProvider';
import { trackPageView } from './services/analytics';
import { startCoreHealthMonitor, stopCoreHealthMonitor } from './services/coreHealthMonitor';
import {
  startInternetStatusListener,
  stopInternetStatusListener,
} from './services/internetStatusListener';
import {
  startWebviewAccountService,
  stopWebviewAccountService,
} from './services/webviewAccountService';
import { persistor, store } from './store';
import { useAppSelector } from './store/hooks';
import { isAccountsFullscreen } from './utils/accountsFullscreen';
import { DEV_FORCE_ONBOARDING } from './utils/config';

// Attach the `webview:event` listener at app boot so background recipe
// events (Google Meet captions → transcript flush, WhatsApp ingest, …)
// are handled even when the user hasn't navigated to /accounts yet.
// Idempotent — the service uses a `started` singleton guard.
// On iOS these services are no-ops (isTauri() webview guard inside each),
// but we call them unconditionally to keep the boot path consistent.
startWebviewAccountService();
startWebviewNotificationsService();
startNativeNotificationsService();
// Connectivity status (#1527): wire navigator.onLine + start core sidecar
// health poll. Both idempotent via internal `started` guards.
startInternetStatusListener();
startCoreHealthMonitor();

export function stopBootServicesForHmr(): void {
  stopWebviewAccountService();
  stopWebviewNotificationsService();
  stopNativeNotificationsService();
  stopInternetStatusListener();
  stopCoreHealthMonitor();
}

if (import.meta.hot) {
  import.meta.hot.dispose(stopBootServicesForHmr);
}

function App() {
  const onMobile = getIsMobile();

  // On mobile (iOS or Android) the SocketProvider would try to connect to the
  // local core HTTP socket, which does not exist on device (the core runs on
  // the remote desktop). Gate it out to prevent spurious connection errors —
  // chat events arrive through TunnelTransport's socket.io relay instead.
  // NOTE: useHumanMascot's subscribeChatEvents() still returns a no-op unsub
  // when the socket is absent — mascot state falls back to 'idle'.
  const socketWrapped = (children: React.ReactNode) =>
    onMobile ? <>{children}</> : <SocketProvider>{children}</SocketProvider>;

  return (
    <Sentry.ErrorBoundary
      fallback={({ error, componentStack, resetError }) => (
        <ErrorFallbackScreen error={error} componentStack={componentStack} onReset={resetError} />
      )}>
      <Provider store={store}>
        <PersistGate loading={<PersistRehydrationScreen />} persistor={persistor}>
          <ThemeProvider>
            <I18nProvider>
              <BootCheckGate>
                <CoreStateProvider>
                  {socketWrapped(
                    <ChatRuntimeProvider>
                      <Router>
                        <CommandProvider>
                          <ServiceBlockingGate>
                            <AppShell />
                            {!onMobile && <DictationHotkeyManager />}
                            {!onMobile && <LocalAIDownloadSnackbar />}
                            {!onMobile && <AppUpdatePrompt />}
                          </ServiceBlockingGate>
                        </CommandProvider>
                      </Router>
                    </ChatRuntimeProvider>
                  )}
                </CoreStateProvider>
              </BootCheckGate>
            </I18nProvider>
          </ThemeProvider>
        </PersistGate>
      </Provider>
    </Sentry.ErrorBoundary>
  );
}

/** Minimal mobile shell — renders routes only, no desktop chrome. */
function AppShellMobile() {
  return (
    <div className="relative h-screen flex flex-col overflow-hidden bg-[#0f1117]">
      <AppRoutes />
    </div>
  );
}

/**
 * Top-level shell router — chooses mobile or desktop shell at render time.
 * Must NOT call hooks before the branch because each sub-component has its
 * own hook calls that obey the rules-of-hooks within their own scope.
 */
function AppShell() {
  const onMobile = getIsMobile();
  if (onMobile) {
    return <AppShellMobile />;
  }
  return <AppShellDesktop />;
}

/** Desktop inner shell — lives inside the Router so it can use useLocation. */
function AppShellDesktop() {
  const location = useLocation();
  const navigate = useNavigate();
  const { snapshot, isBootstrapping } = useCoreState();
  const activeAccountId = useAppSelector(state => state.accounts.activeAccountId);
  // On /accounts, only the agent view keeps the tab bar + its reserved
  // bottom padding. Any other selected "app" (e.g. WhatsApp) takes the
  // full viewport so the embedded webview goes edge-to-edge.
  const fullscreen = isAccountsFullscreen(location.pathname, activeAccountId);
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

  // Track route changes as anonymous page views.
  useEffect(() => {
    trackPageView(location.pathname);
  }, [location.pathname]);

  return (
    <div className="relative h-screen flex flex-col overflow-hidden">
      <AppBackground />
      <div className="relative z-10 flex-1 flex flex-col overflow-hidden">
        <div className={`flex-1 overflow-y-auto ${fullscreen || onOnboardingRoute ? '' : 'pb-16'}`}>
          <GlobalUpsellBanner />
          <AppRoutes />
        </div>
        {!onOnboardingRoute && <BottomTabBar />}
      </div>
      <OpenhumanLinkModal />
      {/* Hidden Remotion-driven producer for the Meet camera. Mounts a
          640×480 JPEG frame stream to the Rust frame bus while a meet
          call is active; idle no-op otherwise. See
          features/meet/MascotFrameProducer.tsx. */}
      <MascotFrameProducer />
      {/* Post-onboarding Joyride walkthrough — mounted here (outside routes) so
          it persists across tab navigations. Joyride targets span Home + BottomTabBar
          tabs so it must stay mounted while the user moves between routes. */}
      {!isBootstrapping && !onOnboardingRoute && (
        <AppWalkthrough onboarded={!!snapshot.onboardingCompleted} />
      )}
    </div>
  );
}

export default App;
