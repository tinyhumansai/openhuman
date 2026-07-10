/**
 * AppRoutesIOS — routes for the iOS + Android app targets.
 *
 * The filename is iOS-historic; the routes apply to every mobile target.
 *
 * Two phases:
 *   1. Unpaired — /pair only. QR scan binds the phone to a desktop core,
 *      writes a profile to profileStore, then redirects to /human.
 *   2. Paired — /human, /chat, /settings/* are reachable. A mobile tab bar
 *      sits at the bottom of the viewport. Any unknown path falls back to
 *      /human. The existing desktop screens (HumanPage, Accounts, Settings)
 *      are reused as-is; they call core RPC through the TransportManager
 *      bound to the saved profile.
 */
import debug from 'debug';
import { type FC } from 'react';
import { Navigate, Route, Routes } from 'react-router-dom';

import MobileTabBar from './components/ios/MobileTabBar';
import HumanPage from './features/human/HumanPage';
import Accounts from './pages/Accounts';
import { PairScreen } from './pages/ios/PairScreen';
import Settings from './pages/Settings';
import { listProfiles } from './services/transport/profileStore';

const log = debug('mobile:routes');

const isPaired = (): boolean => listProfiles().length > 0;

const IOSDefaultRedirect: FC = () => {
  const paired = isPaired();
  log('[mobile] default redirect paired=%s', paired);
  return <Navigate to={paired ? '/human' : '/pair'} replace />;
};

/** Wraps a paired-state route with the mobile tab bar. */
const MobileShell: FC<{ children: React.ReactNode }> = ({ children }) => (
  <div className="relative h-screen flex flex-col overflow-hidden">
    <div className="flex-1 overflow-hidden">{children}</div>
    <MobileTabBar />
  </div>
);

/** Bounces to /pair when no profile exists; otherwise renders children. */
const RequirePairing: FC<{ children: React.ReactNode }> = ({ children }) => {
  if (!isPaired()) {
    log('[mobile] no pairing — redirecting to /pair');
    return <Navigate to="/pair" replace />;
  }
  return <MobileShell>{children}</MobileShell>;
};

const AppRoutesIOS: FC = () => {
  return (
    <Routes>
      {/* Unpaired entry — QR scan handshake. */}
      <Route path="/pair" element={<PairScreen />} />

      {/* Surfaced pages on iOS: Human, Chat, Settings. */}
      <Route
        path="/human"
        element={
          <RequirePairing>
            <HumanPage />
          </RequirePairing>
        }
      />
      <Route
        path="/chat/:threadId?"
        element={
          <RequirePairing>
            <Accounts />
          </RequirePairing>
        }
      />
      <Route
        path="/settings/*"
        element={
          <RequirePairing>
            <Settings />
          </RequirePairing>
        }
      />

      <Route path="*" element={<IOSDefaultRedirect />} />
    </Routes>
  );
};

export default AppRoutesIOS;
