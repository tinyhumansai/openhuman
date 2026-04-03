import * as Sentry from '@sentry/react';
import { Provider } from 'react-redux';
import { HashRouter as Router } from 'react-router-dom';
import { PersistGate } from 'redux-persist/integration/react';

import AppRoutes from './AppRoutes';
import BottomTabBar from './components/BottomTabBar';
import ServiceBlockingGate from './components/daemon/ServiceBlockingGate';
import DictationOverlay from './components/dictation/DictationOverlay';
import ErrorFallbackScreen from './components/ErrorFallbackScreen';
import LocalAIDownloadSnackbar from './components/LocalAIDownloadSnackbar';
import OnboardingOverlay from './components/OnboardingOverlay';
import SocketProvider from './providers/SocketProvider';
import UserProvider from './providers/UserProvider';
import { tagErrorSource } from './services/errorReportQueue';
import { persistor, store } from './store';
import { syncMemoryClientToken } from './utils/tauriCommands';

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
        <PersistGate
          loading={null}
          persistor={persistor}
          onBeforeLift={() => {
            const token = store.getState().auth.token;
            console.info('[memory] PersistGate onBeforeLift: token_present=%s', !!token);
            if (token) {
              // Do not block initial render on core/memory availability.
              void syncMemoryClientToken(token);
            }
          }}>
          <UserProvider>
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
                  <DictationOverlay />
                  <LocalAIDownloadSnackbar />
                </ServiceBlockingGate>
              </Router>
            </SocketProvider>
          </UserProvider>
        </PersistGate>
      </Provider>
    </Sentry.ErrorBoundary>
  );
}

export default App;
