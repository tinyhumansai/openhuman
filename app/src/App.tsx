import * as Sentry from '@sentry/react';
import { useEffect, useRef } from 'react';
import { Provider } from 'react-redux';
import {
  HashRouter as Router,
  useLocation,
  useNavigate,
  useNavigationType,
} from 'react-router-dom';
import { PersistGate } from 'redux-persist/integration/react';

import AppRoutes from './AppRoutes';
import WebviewHost from './components/accounts/WebviewHost';
import AnnouncementGate from './components/Announcement/AnnouncementGate';
import AppBackground from './components/AppBackground';
import AppUpdatePrompt from './components/AppUpdatePrompt';
import BootCheckGate from './components/BootCheckGate/BootCheckGate';
import CommandProvider from './components/commands/CommandProvider';
import ServiceBlockingGate from './components/daemon/ServiceBlockingGate';
import DictationHotkeyManager from './components/DictationHotkeyManager';
import ErrorFallbackScreen from './components/ErrorFallbackScreen';
import HarnessInitOverlay from './components/InitProgressScreen/HarnessInitOverlay';
import KeyringConsentOverlay from './components/keyring/KeyringConsentOverlay';
import AppSidebar from './components/layout/shell/AppSidebar';
import RootShellLayout from './components/layout/shell/RootShellLayout';
import { SidebarSlotProvider } from './components/layout/shell/SidebarSlot';
import LocalAIDownloadSnackbar from './components/LocalAIDownloadSnackbar';
import SecretPromptDialog from './components/mcp-setup/SecretPromptDialog';
import OpenhumanLinkModal from './components/OpenhumanLinkModal';
import PersistRehydrationScreen from './components/PersistRehydrationScreen';
import PttHotkeyManager from './components/PttHotkeyManager';
import SecurityBanner from './components/SecurityBanner';
import SettingsModal from './components/settings/modal/SettingsModal';
import { resolveSettingsOverlay } from './components/settings/modal/settingsOverlay';
import GlobalUpsellBanner from './components/upsell/GlobalUpsellBanner';
import UserErrorCenter from './components/userErrors/UserErrorCenter';
import AppWalkthrough from './components/walkthrough/AppWalkthrough';
import { MascotFrameProducer } from './features/meet/MascotFrameProducer';
import { useNotchBootSync } from './hooks/useNotchBootSync';
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
  hideWebviewAccount,
  startWebviewAccountService,
  stopWebviewAccountService,
} from './services/webviewAccountService';
import { persistor, store } from './store';
import { setActiveAccount } from './store/accountsSlice';
import { useAppDispatch, useAppSelector } from './store/hooks';
import { AGENT_ACCOUNT_ID } from './utils/accountsFullscreen';
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

  /*
   * @generated-source:provider-chain
   * Authoritative top-level provider / gate nesting for the desktop shell,
   * outermost first. Keep this list in sync with the JSX returned below;
   * `scripts/generate-architecture-docs.mjs` renders it into
   * `gitbooks/developing/architecture/frontend.md` and CI (`pnpm docs:check`)
   * fails if the doc drifts. Refresh the doc with `pnpm docs:generate`.
   * Format per line: `<order>. <Component> — <role>` (role must not contain " — ").
   * 1. Sentry.ErrorBoundary — Crash boundary; renders ErrorFallbackScreen
   * 2. Provider — Redux store; enables useAppSelector / dispatch app-wide
   * 3. PersistGate — Holds UI until persisted Redux slices rehydrate
   * 4. ThemeProvider — Theme tokens and dark-mode handling
   * 5. I18nProvider — Localization context consumed via useT
   * 6. BootCheckGate — Blocks render until the core boot snapshot resolves
   * 7. CoreStateProvider — Core app snapshot: auth, session, onboarding state
   * 8. SocketProvider — Core socket.io events; desktop only (mobile uses the TunnelTransport relay)
   * 9. ChatRuntimeProvider — Chat runtime events, tool timeline, and approvals
   * 10. Router — HashRouter navigation for all routes
   * 11. CommandProvider — Command palette context
   * 12. ServiceBlockingGate — Blocks the shell until required services are configured
   * @end-source:provider-chain
   */
  return (
    <Sentry.ErrorBoundary
      fallback={({ error, componentStack, resetError, eventId }) => (
        <ErrorFallbackScreen
          error={error}
          componentStack={componentStack}
          eventId={eventId}
          onReset={resetError}
        />
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
                            <SecurityBanner />
                            {!onMobile && <DictationHotkeyManager />}
                            {!onMobile && <PttHotkeyManager />}
                            {!onMobile && <LocalAIDownloadSnackbar />}
                            {!onMobile && <AppUpdatePrompt />}
                            <KeyringConsentOverlay />
                            <HarnessInitOverlay />
                            <AnnouncementGate />
                            <SecretPromptDialog />
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
export function AppShellDesktop() {
  const location = useLocation();
  const navigate = useNavigate();
  const dispatch = useAppDispatch();
  const { snapshot, isBootstrapping } = useCoreState();
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
        `[onboarding-gate] redirecting ${location.pathname} -> /chat (onboarding complete)`
      );
      navigate('/chat', { replace: true });
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

  // Hide the active connected-app webview when we navigate away from the chat
  // surface. Provider CEF selection is intentionally route-independent; any
  // real route change clears that high-level selection so the native view
  // cannot linger over the newly-routed page.
  const activeAccountId = useAppSelector(state => state.accounts.activeAccountId);
  const accountsById = useAppSelector(state => state.accounts.accounts);
  const accountsOverlayOpen = useAppSelector(state => state.accounts.overlayOpen);
  const previousPathRef = useRef(location.pathname);
  useEffect(() => {
    if (
      location.pathname !== previousPathRef.current &&
      activeAccountId &&
      activeAccountId !== AGENT_ACCOUNT_ID
    ) {
      void hideWebviewAccount(activeAccountId);
      dispatch(setActiveAccount(AGENT_ACCOUNT_ID));
    }
    previousPathRef.current = location.pathname;
  }, [dispatch, location.pathname, activeAccountId]);

  // Sync the notch indicator to the persisted always-on listening state once
  // the core is ready (once per boot). Extracted to a hook so it's testable.
  useNotchBootSync(isBootstrapping);

  const scrollRef = useRef<HTMLDivElement>(null);
  const navType = useNavigationType();

  useEffect(() => {
    if (navType !== 'POP') {
      scrollRef.current?.scrollTo(0, 0);
    }
  }, [location.pathname, navType]);

  // Routes that own the full viewport with no app chrome: the public
  // welcome/login screens, the onboarding stepper, and any pre-auth state.
  // Everything else renders inside the root two-pane shell (sidebar + main).
  const token = snapshot.sessionToken;
  const onHiddenChromePath = ['/', '/login'].some(
    path => location.pathname === path || location.pathname.startsWith(`${path}/`)
  );
  const chromeless = !token || onOnboardingRoute || onHiddenChromePath;

  // Desktop Settings is a modal overlay (the backgroundLocation pattern): when
  // the URL is a settings path we keep rendering the page *behind* it
  // (`baseLocation`) and mount <SettingsModal/> on top (z-50 portal), which sits
  // above the provider WebviewHost overlay (z-30) below.
  const { settingsOpen, baseLocation } = resolveSettingsOverlay(location);

  const activeProviderAccount =
    activeAccountId && activeAccountId !== AGENT_ACCOUNT_ID
      ? (accountsById[activeAccountId] ?? null)
      : null;

  const content = (
    <div ref={scrollRef} className="relative h-full overflow-y-auto">
      <GlobalUpsellBanner />
      <AppRoutes location={baseLocation} />
      {activeProviderAccount && !accountsOverlayOpen && (
        <div className="absolute inset-0 z-30">
          <WebviewHost
            accountId={activeProviderAccount.id}
            provider={activeProviderAccount.provider}
          />
        </div>
      )}
    </div>
  );

  return (
    <SidebarSlotProvider>
      <div className="relative h-screen flex flex-col overflow-hidden">
        <AppBackground />
        <div className="relative z-10 flex-1 min-h-0 flex flex-col overflow-hidden">
          {chromeless ? (
            content
          ) : (
            <RootShellLayout sidebar={<AppSidebar />}>{content}</RootShellLayout>
          )}
        </div>
        {/* Desktop Settings modal — mounted over whatever page is rendered
            beneath when the URL is a settings path. */}
        {settingsOpen && !chromeless && <SettingsModal />}
        <OpenhumanLinkModal />
        {/* User-actionable runtime errors (#3931): a first-class panel for
            expected user states (insufficient BYO credits, managed-budget
            exhaustion). Mounted outside the routes so entries survive route
            changes and background-job completion. */}
        <UserErrorCenter />
        {/* Hidden Remotion-driven producer for the Meet camera. Mounts a
            640×480 JPEG frame stream to the Rust frame bus while a meet
            call is active; idle no-op otherwise. See
            features/meet/MascotFrameProducer.tsx. */}
        <MascotFrameProducer />
        {/* Post-onboarding Joyride walkthrough — mounted here (outside routes) so
            it persists across tab navigations. Joyride targets span Home + the
            sidebar nav so it must stay mounted while the user moves between routes. */}
        {!isBootstrapping && !onOnboardingRoute && (
          <AppWalkthrough onboarded={!!snapshot.onboardingCompleted} />
        )}
      </div>
    </SidebarSlotProvider>
  );
}

export default App;
