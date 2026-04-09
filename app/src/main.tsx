// IMPORTANT: Polyfills must be imported FIRST
import { isTauri as tauriRuntimeAvailable } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import React from 'react';
import ReactDOM from 'react-dom/client';

import App from './App';
import ErrorReportNotification from './components/ErrorReportNotification';
import './index.css';
import { getCoreStateSnapshot } from './lib/coreState/store';
import OverlayApp from './overlay/OverlayApp';
import './polyfills';
import { initSentry } from './services/analytics';
import { setStoreForApiClient } from './services/apiClient';
import { setupDesktopDeepLinkListener } from './utils/desktopDeepLinkListener';

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

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>{isOverlayWindow ? <OverlayApp /> : <App />}</React.StrictMode>
);

if (!isOverlayWindow) {
  // Mount error notification in an isolated React root so it survives App crashes.
  const errorRoot = document.createElement('div');
  errorRoot.id = 'error-report-root';
  document.body.appendChild(errorRoot);
  ReactDOM.createRoot(errorRoot).render(<ErrorReportNotification />);
}
