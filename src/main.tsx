// IMPORTANT: Polyfills must be imported FIRST
import React from 'react';
import ReactDOM from 'react-dom/client';

import App from './App';
import './index.css';
import './polyfills';
import { initSentry } from './services/analytics';
import { setupDesktopDeepLinkListener } from './utils/desktopDeepLinkListener';

// Initialize Sentry early (before React renders)
initSentry();

// Deep link listener — try/catch handles non-Tauri environments
setupDesktopDeepLinkListener().catch(err => {
  console.error('[DeepLink] setup error:', err);
});

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
