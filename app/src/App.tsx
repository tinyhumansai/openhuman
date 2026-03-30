import * as Sentry from '@sentry/react';
import { Provider } from 'react-redux';
import { HashRouter as Router } from 'react-router-dom';
import { PersistGate } from 'redux-persist/integration/react';

import AppRoutes from './AppRoutes';
import ErrorFallbackScreen from './components/ErrorFallbackScreen';
import MiniSidebar from './components/MiniSidebar';
import ServiceBlockingGate from './components/daemon/ServiceBlockingGate';
import AIProvider from './providers/AIProvider';
import SkillProvider from './providers/SkillProvider';
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
              <AIProvider>
                <SkillProvider>
                  <Router>
                    <ServiceBlockingGate>
                      <div className="relative h-screen flex flex-col overflow-hidden">
                        <div className="flex-1 flex overflow-hidden">
                          <MiniSidebar />
                          <div className="flex flex-col flex-1 relative overflow-hidden">
                            <div className="flex-1 overflow-y-auto">
                              <AppRoutes />
                            </div>
                            <div className="pointer-events-none flex-shrink-0 flex justify-center z-50">
                              <div className="w-full px-3 py-1.5 text-[9px] uppercase tracking-[0.18em] text-white/40 text-center bg-[#000]">
                                OpenHuman is in early beta
                              </div>
                            </div>
                          </div>
                        </div>
                      </div>
                    </ServiceBlockingGate>
                  </Router>
                </SkillProvider>
              </AIProvider>
            </SocketProvider>
          </UserProvider>
        </PersistGate>
      </Provider>
    </Sentry.ErrorBoundary>
  );
}

export default App;
