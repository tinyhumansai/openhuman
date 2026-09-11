// IMPORTANT: Polyfills must be imported FIRST
import { getCurrentWindow } from '@tauri-apps/api/window';
import 'katex/dist/katex.min.css';
import React from 'react';
import ReactDOM from 'react-dom/client';

import App from './App';
import './index.css';
import { installAutoHideScrollbars } from './lib/autoHideScrollbars';
import { getCoreStateSnapshot } from './lib/coreState/store';
import MascotWindowApp from './mascot/MascotWindowApp';
import NotchApp from './notch/NotchApp';
import OverlayApp from './overlay/OverlayApp';
import './polyfills';
import { initGA, initSentry, startUiInteractionTracking, trackEvent } from './services/analytics';
import { setStoreForApiClient } from './services/apiClient';
import { primeActiveUserId } from './store/userScopedStorage';
import './styles/code-highlight.css';
import { resolveActiveUserBootstrap } from './utils/bootstrapActiveUser';
import { APP_VERSION } from './utils/config';
import { getStoredCoreMode } from './utils/configPersistence';
import { setupDesktopDeepLinkListener } from './utils/desktopDeepLinkListener';
import { missingHashRedirectTarget } from './utils/hashRouterBootstrap';
import { installIpcTransportFallback } from './utils/ipcTransportFallback';
import { getActiveUserIdFromCore } from './utils/tauriCommands';
import { isTauri as tauriRuntimeAvailable } from './utils/tauriCommands/common';

// Must run before anything can `invoke()`. Tauri's vendored IPC bootstrap
// falls back to `window.ipc.postMessage(...)` once the `ipc://` custom-protocol
// fetch rejects even once, and CEF never wires `window.ipc` — that dereference
// is Sentry TAURI-REACT-6 / #5155. Installing a working fallback here makes the
// undefined access impossible AND lets the latched fallback keep serving IPC
// instead of bricking the session. Module-body position is early enough: ESM
// hoists every import above this line, and no imported module invokes at
// import time — the first real `invoke()` happens in a React effect.
installIpcTransportFallback();

setStoreForApiClient(() => getCoreStateSnapshot().snapshot.sessionToken);

// The floating mascot is hosted in a native macOS NSPanel + WKWebView
// that lives OUTSIDE Tauri's runtime (the vendored tauri-cef can't render
// transparent windowed-mode browsers). That webview can't read a Tauri
// window label, so the Rust shell appends `?window=mascot` to the URL it
// loads. Detect it via the URL param so we can skip `getCurrentWindow()`
// — which would either throw or trigger the CEF IPC-bootstrap gap that
// `tauriRuntimeAvailable()` (= the hardened `isTauri()`) now guards
// against by reading `window.__TAURI_INTERNALS__.invoke`.
const urlWindowParam = (() => {
  try {
    return new URLSearchParams(window.location.search).get('window');
  } catch {
    return null;
  }
})();
const isMascotWindow = urlWindowParam === 'mascot';
const isNotchWindow = urlWindowParam === 'notch';
const currentWindowLabel = isMascotWindow
  ? 'mascot'
  : isNotchWindow
    ? 'notch'
    : tauriRuntimeAvailable()
      ? getCurrentWindow().label
      : 'main';
const isOverlayWindow = currentWindowLabel === 'overlay';
const isStandaloneWindow = isOverlayWindow || isMascotWindow || isNotchWindow;

const ensureDefaultHashRoute = () => {
  const hash = window.location.hash;
  if (!hash || hash === '#') {
    window.location.replace(
      missingHashRedirectTarget(window.location.pathname, window.location.search)
    );
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
  startUiInteractionTracking();
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
  // Reveal scrollbars only while a pane is actually scrolling. Installed here
  // rather than in a component so it covers every window entry (main, mascot,
  // notch, overlay) with one document-level capture listener, and so panes
  // mounted later need no wiring. Lives for the document's lifetime.
  installAutoHideScrollbars();

  const root = ReactDOM.createRoot(document.getElementById('root') as HTMLElement);
  const tree = isMascotWindow ? (
    <MascotWindowApp />
  ) : isNotchWindow ? (
    <NotchApp />
  ) : isOverlayWindow ? (
    <OverlayApp />
  ) : (
    <App />
  );
  root.render(<React.StrictMode>{tree}</React.StrictMode>);
}

// Decide which source (Rust IPC vs preserved localStorage seed) primes
// `userScopedStorage` at boot. Cloud mode and standalone native windows
// both fall through to `primeActiveUserId(null)`, which preserves whatever
// `setActiveUserId(...)` wrote in a prior session — see
// `resolveActiveUserBootstrap` for the full rationale (#4545, #900).
const activeUserBootstrap = resolveActiveUserBootstrap({
  isStandaloneNativeWindow: isMascotWindow || isNotchWindow,
  coreMode: getStoredCoreMode(),
  getActiveUserIdFromCore,
});

activeUserBootstrap
  .then(id => primeActiveUserId(id))
  .catch(() => primeActiveUserId(null))
  .finally(bootRender);
