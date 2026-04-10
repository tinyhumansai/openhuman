import * as Sentry from '@sentry/react';
import { Provider } from 'react-redux';
import { HashRouter as Router } from 'react-router-dom';
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
import CoreStateProvider from './providers/CoreStateProvider';
import SocketProvider from './providers/SocketProvider';
import { tagErrorSource } from './services/errorReportQueue';
import { persistor, store } from './store';

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
              <Router>
                <ServiceBlockingGate>
                  <div className="relative h-screen flex flex-col overflow-hidden">
                    <MeshGradient />
                    <div className="app-dotted-canvas relative z-10 flex-1 flex flex-col overflow-hidden">
                      <div className="flex-1 overflow-y-auto pb-16">
                        <GlobalUpsellBanner />
                        <AppRoutes />
                      </div>
                      <BottomTabBar />
                    </div>
                  </div>
                  <OnboardingOverlay />
                  <DictationHotkeyManager />
                  <LocalAIDownloadSnackbar />
                </ServiceBlockingGate>
              </Router>
            </SocketProvider>
          </CoreStateProvider>
        </PersistGate>
      </Provider>
    </Sentry.ErrorBoundary>
  );
}

export default App;
