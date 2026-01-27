import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";
import { setupDesktopDeepLinkListener } from "./utils/desktopDeepLinkListener";

// Start listening for deep-link events as early as possible.
void setupDesktopDeepLinkListener();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
