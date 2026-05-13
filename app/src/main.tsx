// IMPORTANT: Polyfills must be imported FIRST
import { isTauri as tauriRuntimeAvailable } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import React from 'react';
import ReactDOM from 'react-dom/client';

import App from './App';
import './index.css';
import { getCoreStateSnapshot } from './lib/coreState/store';
import MascotWindowApp from './mascot/MascotWindowApp';
import OverlayApp from './overlay/OverlayApp';
import './polyfills';
import { initGA, initSentry, trackEvent } from './services/analytics';
import { setStoreForApiClient } from './services/apiClient';
import { primeActiveUserId } from './store/userScopedStorage';
import { APP_VERSION } from './utils/config';
import { setupDesktopDeepLinkListener } from './utils/desktopDeepLinkListener';
import { getActiveUserIdFromCore } from './utils/tauriCommands';

setStoreForApiClient(() => getCoreStateSnapshot().snapshot.sessionToken);

// The floating mascot is hosted in a native macOS NSPanel + WKWebView
// that lives OUTSIDE Tauri's runtime (the vendored tauri-cef can't render
// transparent windowed-mode browsers). That webview can't read a Tauri
// window label, so the Rust shell appends `?window=mascot` to the URL it
// loads. Detect it before we touch any Tauri APIs.
const urlWindowParam = (() => {
  try {
    return new URLSearchParams(window.location.search).get('window');
  } catch {
    return null;
  }
})();
const isMascotWindow = urlWindowParam === 'mascot';
const currentWindowLabel = isMascotWindow
  ? 'mascot'
  : tauriRuntimeAvailable()
    ? getCurrentWindow().label
    : 'main';
const isOverlayWindow = currentWindowLabel === 'overlay';
const isStandaloneWindow = isOverlayWindow || isMascotWindow;

const ensureDefaultHashRoute = () => {
  const hash = window.location.hash;
  if (!hash || hash === '#') {
    window.location.replace(`${window.location.pathname}${window.location.search}#/`);
    return;
  }
  if (!hash.startsWith('#/')) {
    window.location.hash = '/';
  }
};

// Initialize Sentry and GA early (before React renders)
initSentry();
initGA();
if (!isStandaloneWindow) {
  trackEvent('app_open', { version: APP_VERSION });
}
document.documentElement.dataset.window = currentWindowLabel;

if (!isStandaloneWindow) {
  ensureDefaultHashRoute();

  // Deep link listener — try/catch handles non-Tauri environments
  setupDesktopDeepLinkListener().catch(err => {
    console.error('[DeepLink] setup error:', err);
  });
}

// Prime `userScopedStorage` from the Rust core's `active_user.toml`
// BEFORE redux-persist hydrates. The previous localStorage-only seed was
// bound to the per-user CEF profile dir and went stale across the
// restart-driven user flips that #900 introduced, so the new process
// would read the previous user's namespace, mis-detect a flip, and bounce
// into a second restart. Reading the Rust state up front pins the right
// namespace from the first storage call. (#900)
function bootRender() {
  const root = ReactDOM.createRoot(document.getElementById('root') as HTMLElement);
  const tree = isMascotWindow ? <MascotWindowApp /> : isOverlayWindow ? <OverlayApp /> : <App />;
  root.render(<React.StrictMode>{tree}</React.StrictMode>);
}

// The mascot lives in a native WKWebView (no Tauri IPC), so
// `getActiveUserIdFromCore()` would just reject after a roundtrip and
// delay first paint for nothing. Skip the bootstrap entirely in that
// path — the mascot UI doesn't read user-scoped storage anyway.
const activeUserBootstrap = isMascotWindow
  ? Promise.resolve<string | null>(null)
  : getActiveUserIdFromCore();

activeUserBootstrap
  .then(id => primeActiveUserId(id))
  .catch(() => primeActiveUserId(null))
  .finally(bootRender);
