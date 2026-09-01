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
import { AnalyticsPageTracker } from './components/analytics';
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
import NoticeCenter from './components/notices/NoticeCenter';
import OpenhumanLinkModal from './components/OpenhumanLinkModal';
import PersistRehydrationScreen from './components/PersistRehydrationScreen';
import PttHotkeyManager from './components/PttHotkeyManager';
import SecurityBanner from './components/SecurityBanner';
import AppWalkthrough from './components/walkthrough/AppWalkthrough';
import { useNotchBootSync } from './hooks/useNotchBootSync';
import { I18nProvider } from './lib/i18n/I18nContext';
import {
  startNativeNotificationsService,
  stopNativeNotificationsService,
} from './lib/nativeNotifications';
import { getIsMobile } from './lib/platform';
import ChatRuntimeProvider from './providers/ChatRuntimeProvider';
import CoreStateProvider, { useCoreState } from './providers/CoreStateProvider';
import SocketProvider from './providers/SocketProvider';
import ThemeProvider from './providers/ThemeProvider';
import { startCoreHealthMonitor, stopCoreHealthMonitor } from './services/coreHealthMonitor';
import {
  startInternetStatusListener,
  stopInternetStatusListener,
} from './services/internetStatusListener';
import { persistor, store } from './store';
import { DEV_FORCE_ONBOARDING } from './utils/config';

startNativeNotificationsService();
// Connectivity status (#1527): wire navigator.onLine + start core sidecar
// health poll. Both idempotent via internal `started` guards.
startInternetStatusListener();
startCoreHealthMonitor();

export function stopBootServicesForHmr(): void {
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
                            <AnalyticsPageTracker />
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
  // The workflow graph canvas (`/flows/:id`, `/flows/draft`) used to be listed
  // here too, as "a focused builder — no app sidebar". It is back in the shell:
  // going chromeless cost it the app nav AND the sidebar slot, so the builder
  // had to hand-roll its own 240px run-history rail inside the page (`hidden
  // lg:flex w-60 border-r`) — a second sidebar sitting where the real one would
  // have been. It now projects that rail through `SidebarContent` like every
  // other page, and a user who wants the focused view collapses the sidebar,
  // which is what `collapsible="icon"` is for.
  const chromeless = !token || onOnboardingRoute || onHiddenChromePath;

  const content = (
    <div ref={scrollRef} className="relative h-full overflow-y-auto">
      {/* The plan-usage upsell and the #5324 memory-embedding warning used to
          be full-width banners here, pushing every route down. Both are
          notices in `NoticeCenter` now — see its docs for why. */}
      <AppRoutes />
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
            // Nothing sets `unframed` today. It existed for live CEF provider
            // webviews — WebviewHost handed the Rust side a plain rectangle and
            // CEF composited that child view above the whole HTML layer, so a
            // rounded card under it showed four square corners punching through
            // the radius. That surface was removed upstream along with
            // WebviewHost, so no route needs the escape hatch right now; the
            // prop stays on the primitive for the next full-bleed surface.
            <RootShellLayout sidebar={<AppSidebar />}>{content}</RootShellLayout>
          )}
        </div>
        <OpenhumanLinkModal />
        {/* Every notice the app raises, in one bottom-left FAB: classified
            runtime errors (#3931), the memory-embedding budget (#5324), plan
            usage limits. Mounted outside the routes so entries survive route
            changes and background-job completion. */}
        <NoticeCenter />
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
