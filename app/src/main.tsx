// IMPORTANT: Polyfills must be imported FIRST
import { isTauri as tauriRuntimeAvailable } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import React from 'react';
import ReactDOM from 'react-dom/client';

import App from './App';
import './index.css';
import { getCoreStateSnapshot } from './lib/coreState/store';
import OverlayApp from './overlay/OverlayApp';
import './polyfills';
import { initSentry } from './services/analytics';
import { setStoreForApiClient } from './services/apiClient';
import { primeActiveUserId } from './store/userScopedStorage';
import { setupDesktopDeepLinkListener } from './utils/desktopDeepLinkListener';
import { getActiveUserIdFromCore } from './utils/tauriCommands';

setStoreForApiClient(() => getCoreStateSnapshot().snapshot.sessionToken);

const currentWindowLabel = tauriRuntimeAvailable() ? getCurrentWindow().label : 'main';
const isOverlayWindow = currentWindowLabel === 'overlay';

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

// Initialize Sentry early (before React renders)
initSentry();
document.documentElement.dataset.window = currentWindowLabel;

if (!isOverlayWindow) {
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
  root.render(<React.StrictMode>{isOverlayWindow ? <OverlayApp /> : <App />}</React.StrictMode>);
}

getActiveUserIdFromCore()
  .then(id => primeActiveUserId(id))
  .catch(() => primeActiveUserId(null))
  .finally(bootRender);
