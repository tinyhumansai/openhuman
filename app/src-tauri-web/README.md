## src-tauri-web

This sibling to `src-tauri-mobile/` is the browser-hosted shell profile for
OpenHuman E2E and future web-compatible development.

Scope:

- No CEF runtime
- No embedded provider webviews
- No native windowing, tray, or deep-link plugins
- Frontend talks to a standalone `openhuman-core` over HTTP JSON-RPC

Current entrypoints:

- `pnpm build:web:e2e` builds the browser bundle into `app/dist-web`
- `pnpm test:e2e:web` starts the mock backend, standalone core, and static web
  host, then runs Playwright against the browser build
- `pnpm test:e2e:mega` keeps the CEF/Appium mega-flow on the desktop shell

This folder is intentionally documentation-first for now. The browser shell is
composed from the existing Vite app plus the standalone core runner rather than
another Tauri crate.
