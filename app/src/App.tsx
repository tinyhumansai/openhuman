import * as Sentry from '@sentry/react';
import { Provider } from 'react-redux';
import { HashRouter as Router } from 'react-router-dom';
import { PersistGate } from 'redux-persist/integration/react';

import AppRoutes from './AppRoutes';
import BottomTabBar from './components/BottomTabBar';
import ServiceBlockingGate from './components/daemon/ServiceBlockingGate';
import ErrorFallbackScreen from './components/ErrorFallbackScreen';
import LocalAIDownloadSnackbar from './components/LocalAIDownloadSnackbar';
import OnboardingOverlay from './components/OnboardingOverlay';
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
        <PersistGate loading={null} persistor={persistor}>
          <CoreStateProvider>
            <SocketProvider>
              <Router>
                <ServiceBlockingGate>
                  <div className="relative h-screen flex flex-col overflow-hidden bg-[#F5F5F5]">
                    <div className="flex-1 overflow-y-auto">
                      <AppRoutes />
                    </div>
                    <BottomTabBar />
                  </div>
                  <OnboardingOverlay />
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
