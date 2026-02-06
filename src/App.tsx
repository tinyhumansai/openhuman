import * as Sentry from '@sentry/react';
import { Provider } from 'react-redux';
import { HashRouter as Router } from 'react-router-dom';
import { PersistGate } from 'redux-persist/integration/react';

import AppRoutes from './AppRoutes';
import AIProvider from './providers/AIProvider';
import ModelProvider from './providers/ModelProvider';
import SkillProvider from './providers/SkillProvider';
import SocketProvider from './providers/SocketProvider';
import UserProvider from './providers/UserProvider';
import { persistor, store } from './store';

function App() {
  return (
    <Sentry.ErrorBoundary fallback={<div>Something went wrong.</div>}>
      <Provider store={store}>
        <PersistGate loading={null} persistor={persistor}>
          <ModelProvider>
            <UserProvider>
              <SocketProvider>
                <AIProvider>
                  <SkillProvider>
                    <Router>
                      <div className="relative h-screen flex flex-col overflow-hidden">
                        <div className="pointer-events-none flex-shrink-0 flex justify-center z-50">
                          <div className="bg-black w-full px-3 py-1.5 text-[11px] uppercase tracking-[0.18em] text-white/40 text-center">
                            AlphaHuman is in early beta.
                          </div>
                        </div>
                        <div className="flex-1 overflow-y-auto">
                          <AppRoutes />
                        </div>
                      </div>
                    </Router>
                  </SkillProvider>
                </AIProvider>
              </SocketProvider>
            </UserProvider>
          </ModelProvider>
        </PersistGate>
      </Provider>
    </Sentry.ErrorBoundary>
  );
}

export default App;
