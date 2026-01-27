import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";

// Deep link listener - lazy import to avoid running before Tauri IPC is ready
import('./utils/desktopDeepLinkListener').then(m => {
  m.setupDesktopDeepLinkListener().catch(err => {
    console.error('[DeepLink] setup error:', err);
  });
});

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
