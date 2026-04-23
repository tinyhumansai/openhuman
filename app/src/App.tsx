import * as Sentry from '@sentry/react';
import { Provider } from 'react-redux';
import { HashRouter as Router, useLocation } from 'react-router-dom';
import { PersistGate } from 'redux-persist/integration/react';

import AppRoutes from './AppRoutes';
import BottomTabBar from './components/BottomTabBar';
import ServiceBlockingGate from './components/daemon/ServiceBlockingGate';
import DictationHotkeyManager from './components/DictationHotkeyManager';
import ErrorFallbackScreen from './components/ErrorFallbackScreen';
import LocalAIDownloadSnackbar from './components/LocalAIDownloadSnackbar';
import MeshGradient from './components/MeshGradient';
import OnboardingOverlay from './components/OnboardingOverlay';
import RouteLoadingScreen from './components/RouteLoadingScreen';
import GlobalUpsellBanner from './components/upsell/GlobalUpsellBanner';
import { startNativeNotificationsService } from './lib/nativeNotifications';
import { startWebviewNotificationsService } from './lib/webviewNotifications';
import ChatRuntimeProvider from './providers/ChatRuntimeProvider';
import CoreStateProvider from './providers/CoreStateProvider';
import SocketProvider from './providers/SocketProvider';
import { tagErrorSource } from './services/errorReportQueue';
import { startWebviewAccountService } from './services/webviewAccountService';
import { persistor, store } from './store';
import { useAppSelector } from './store/hooks';
import { isAccountsFullscreen } from './utils/accountsFullscreen';

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
                  <ServiceBlockingGate>
                    <AppShell />
                    <OnboardingOverlay />
                    <DictationHotkeyManager />
                    <LocalAIDownloadSnackbar />
                  </ServiceBlockingGate>
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
  const activeAccountId = useAppSelector(state => state.accounts.activeAccountId);
  // On /accounts, only the agent view keeps the tab bar + its reserved
  // bottom padding. Any other selected "app" (e.g. WhatsApp) takes the
  // full viewport so the embedded webview goes edge-to-edge.
  const fullscreen = isAccountsFullscreen(location.pathname, activeAccountId);

  return (
    <div className="relative h-screen flex flex-col overflow-hidden">
      <MeshGradient />
      <div className="app-dotted-canvas relative z-10 flex-1 flex flex-col overflow-hidden">
        <div className={`flex-1 overflow-y-auto ${fullscreen ? '' : 'pb-16'}`}>
          <GlobalUpsellBanner />
          <AppRoutes />
        </div>
        <BottomTabBar />
      </div>
    </div>
  );
}

export default App;
