/**
 * AppRoutesIOS — routes for the iOS + Android app targets.
 *
 * The filename is iOS-historic; the routes apply to every mobile target.
 *
 * Two phases:
 *   1. Unpaired — /pair only. QR scan binds the phone to a desktop core,
 *      writes a profile to profileStore, then redirects to /human.
 *   2. Paired — /human, /chat and /settings/* are reachable. A mobile tab bar
 *      sits at the bottom of the viewport. Any unknown path falls back to
 *      /human. The existing desktop screens (HumanPage, Accounts, Settings) are
 *      reused as-is; they call core RPC through the TransportManager bound to
 *      the saved profile.
 */
import debug from 'debug';
import { type FC, useEffect, useState } from 'react';
import { Navigate, Route, Routes, useLocation, useNavigate } from 'react-router-dom';

import MobileTabBar from './components/ios/MobileTabBar';
import HumanPage from './features/human/HumanPage';
import { useT } from './lib/i18n/I18nContext';
import Accounts from './pages/Accounts';
import { PairScreen } from './pages/ios/PairScreen';
import Settings from './pages/Settings';
import { getActiveCoreTransport, setActiveCoreTransport } from './services/coreRpcClient';
import { listProfiles } from './services/transport/profileStore';
import { createTransportManager } from './services/transport/TransportManager';
import { BACKEND_URL } from './utils/config';

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

const TransportBootstrapError: FC = () => {
  const { t } = useT();
  const navigate = useNavigate();

  return (
    <div className="flex min-h-screen flex-col items-center justify-center gap-6 bg-[#0f1117] px-6 text-center text-content-inverted">
      <p className="max-w-sm text-sm text-red-400">{t('iosPair.error.connectionFailed')}</p>
      <button
        type="button"
        onClick={() => navigate('/pair', { replace: true })}
        className="rounded-xl bg-[#4A83DD] px-6 py-3 text-sm text-content-inverted active:opacity-80">
        {t('iosPair.scanQrCode')}
      </button>
    </div>
  );
};

/** Bind a persisted mobile profile before paired screens issue core RPCs. */
const MobileTransportBootstrap: FC<{ children: React.ReactNode }> = ({ children }) => {
  const location = useLocation();
  const [ready, setReady] = useState(false);
  const [bindingFailed, setBindingFailed] = useState(false);

  useEffect(() => {
    // Profiles can change while the pairing screen is mounted. Read the
    // current store on each navigation so returning to a paired route binds
    // the profile created by the latest pairing attempt.
    const profile = listProfiles().at(-1) ?? null;
    if (getActiveCoreTransport()) {
      setBindingFailed(false);
      setReady(true);
      return;
    }
    if (!profile?.kind) {
      setActiveCoreTransport(null);
      setBindingFailed(false);
      setReady(true);
      return;
    }

    let disposed = false;
    const manager = createTransportManager(profile, { backendSocketUrl: BACKEND_URL });
    void manager
      .getTransport()
      .then(transport => {
        if (disposed) return;
        return transport.isHealthy().then(healthy => {
          if (disposed) return;
          if (!healthy) throw new Error('persisted transport is unhealthy');
          setActiveCoreTransport(transport);
          setBindingFailed(false);
          setReady(true);
          log('[mobile] bound persisted transport kind=%s', transport.kind);
        });
      })
      .catch(error => {
        if (!disposed) {
          setActiveCoreTransport(null);
          setBindingFailed(true);
          setReady(false);
        }
        log(
          '[mobile] persisted transport binding failed: %s',
          error instanceof Error ? error.message : 'unknown transport error'
        );
      });

    return () => {
      disposed = true;
      void manager.close();
    };
  }, [location.pathname]);

  if (location.pathname === '/pair') return <>{children}</>;
  if (bindingFailed) return <TransportBootstrapError />;
  return ready ? <>{children}</> : null;
};

const AppRoutesIOS: FC = () => {
  return (
    <MobileTransportBootstrap>
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
    </MobileTransportBootstrap>
  );
};

export default AppRoutesIOS;
