import * as Sentry from '@sentry/react';
import { Provider } from 'react-redux';
import { HashRouter as Router } from 'react-router-dom';
import { PersistGate } from 'redux-persist/integration/react';

import AppRoutes from './AppRoutes';
import AIProvider from './providers/AIProvider';
import SkillProvider from './providers/SkillProvider';
import SocketProvider from './providers/SocketProvider';
import UserProvider from './providers/UserProvider';
import { persistor, store } from './store';

function App() {
  return (
    <Sentry.ErrorBoundary fallback={<div>Something went wrong.</div>}>
      <Provider store={store}>
        <PersistGate loading={null} persistor={persistor}>
          <UserProvider>
            <SocketProvider>
              <AIProvider>
                <SkillProvider>
                  <Router>
                    <div className="relative min-h-screen">
                      <div className="pointer-events-none fixed inset-x-0 top-0 flex justify-center z-50">
                        <div className="bg-black w-full px-3 py-1.5 text-[11px] uppercase tracking-[0.18em] text-white/40 text-center">
                          AlphaHuman is in early beta.
                        </div>
                      </div>
                      <div className="pt-7">
                        <AppRoutes />
                      </div>
                    </div>
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
