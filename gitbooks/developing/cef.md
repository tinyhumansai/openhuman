---
description: >-
  Why OpenHuman ships its own Chromium runtime, what we use it for today, and
  what the same CDP surface unlocks next.
icon: chrome
---

# Chromium Embedded Framework

OpenHuman doesn't run on the platform's built-in webview. It ships its own **Chromium Embedded Framework (CEF) runtime** via a fork of `tauri-runtime`, and that single decision is load-bearing for almost every "OpenHuman knows what's happening in your tools" feature in the product.

This page explains why CEF is in the bundle, what the codebase uses it for today, and where the same surface could go.

## Why CEF instead of a stock webview

Stock Tauri uses each platform's native webview. WKWebView on macOS, WebView2 on Windows, WebKitGTK on Linux. Those work fine for rendering the OpenHuman app itself. They have one fatal limitation for our use case: **none of them expose Chrome DevTools Protocol (CDP)**.

CDP is the load-bearing primitive. Every "watch what's happening inside Slack / WhatsApp / Telegram / Discord / Meet" feature in OpenHuman talks to those embedded apps via CDP, not via injected JavaScript. CDP gives us:

* `Target.getTargets` to discover every page and service worker.
* `IndexedDB.requestDatabaseNames` / `requestDatabase` / `requestData` to walk a third-party app's local storage.
* `DOMSnapshot.captureSnapshot` for read-only DOM inspection that doesn't trip framework reactivity.
* `Runtime.evaluate` for ephemeral one-shot reads (a single fixed JSON serializer, never a persistent bridge).
* `Page.addScriptToEvaluateOnNewDocument` for the small number of cases where we genuinely need a renderer-side shim before page JS runs.

Stock webviews can't give us any of that. So we vendor CEF.

The vendored runtime lives at [`app/src-tauri/vendor/tauri-cef/`](https://github.com/tinyhumansai/openhuman/tree/main/app/src-tauri/vendor/tauri-cef) (forked from the upstream `tauri-cef` branch onto `tinyhumansai/tauri-cef:feat/cef-notification-intercept`, currently CEF 146.4.1). Every Tauri crate is patched at `app/src-tauri/Cargo.toml` via `[patch.crates-io]` to point at this fork. The vendored `cargo-tauri` CLI bundles Chromium correctly into `Contents/Frameworks/`; stock `@tauri-apps/cli` produces a broken bundle that panics in `cef::library_loader::LibraryLoader::new`. [`scripts/ensure-tauri-cli.sh`](../../scripts/ensure-tauri-cli.sh) reinstalls the vendored CLI whenever the fork is newer than the installed binary.

## What CEF is used for today

### Embedded third-party webviews

Every connected provider that runs as a hosted web app gets its own child CEF webview:

* WhatsApp Web
* Telegram Web
* Slack
* Discord
* Google Meet
* LinkedIn
* Gmail
* Zoom
* browserscan

Per-account storage is isolated to `{app_local_data_dir}/webview_accounts/{id}/`. Two Slack workspaces, two browser profiles. Code: [`app/src-tauri/src/webview_accounts/mod.rs`](../../app/src-tauri/src/webview_accounts/mod.rs).

### CDP-driven scanners

Each provider has a **scanner module** in [`app/src-tauri/src/`](https://github.com/tinyhumansai/openhuman/tree/main/app/src-tauri/src). Every scanner holds a long-lived WebSocket to CEF's `--remote-debugging-port=19222` and ticks on a fixed schedule:

| Scanner            | Cadence                         | What it does                                                         |
| ------------------ | ------------------------------- | -------------------------------------------------------------------- |
| `whatsapp_scanner` | 2s DOM tick + 30s full IDB walk | Reads message stores, pulls media metadata                           |
| `telegram_scanner` | Same                            | Plus QR-login hand-off to native Telegram Desktop                    |
| `slack_scanner`    | 30s IDB walk                    | Pure IDB - no DOM scrape needed                                      |
| `discord_scanner`  | Periodic                        | Channel + DM state via CDP                                           |
| `meet_scanner`     | Periodic                        | Live captions + participant state during calls                       |
| `imessage_scanner` | Periodic                        | **No webview.** Reads `~/Library/Messages/chat.db` directly on macOS |

Each scan emits `webview:event` payloads and POSTs `openhuman.memory_doc_ingest` straight to the core RPC, so memory grows whether the UI window is open or backgrounded.

### Google Meet mascot camera

The flashiest CEF trick. The Meet agent doesn't just _attend_ a meeting, it **broadcasts** itself as a camera. This works because CEF lets us:

1. Inject a tiny bridge (`camera_bridge.js`) via `Page.addScriptToEvaluateOnNewDocument` before any Meet code runs.
2. Override `navigator.mediaDevices.getUserMedia` so it returns a `MediaStream` from a hidden 640×480 canvas instead of a real camera.
3. Render the mascot SVG on that canvas, swapping mood states (idle, thinking, talking) via `window.__openhumanSetMood(...)` driven from Rust over CDP.

There's also a build-time path that rasterizes the mascot SVG to Y4M and uses CEF's native `--use-file-for-fake-video-capture` flag, a fully native fake-camera source with no JS at all.

Code: [`app/src-tauri/src/meet_video/`](https://github.com/tinyhumansai/openhuman/tree/main/app/src-tauri/src/meet_video).

### Native notification interception

The fork at `feat/cef-notification-intercept` adds renderer-side shims for `Notification.permission`, `Notification.requestPermission()`, and `navigator.permissions.query({name: "notifications"})`. These now install in the real `tauri-runtime-cef` path on every runtime code path, so when Slack checks if it can show notifications, the answer is consistent with what CEF's permission callbacks already granted.

This is the bulk of `docs/TAURI_CEF_FINDINGS_AND_CHANGES.md`. It's why Slack stops asking the same permission five times in a session.

## The "no new JS injection" rule

The rule is documented in [`CLAUDE.md`](../../CLAUDE.md): **migrated providers load with zero injected JavaScript**. All scraping happens natively over CDP from the scanner side.

This matters because anything host-controlled that runs inside a third-party origin is an attack-surface liability. A persistent JS bridge inside Slack is one Slack update away from breaking, and one mistake away from leaking the bridge to attacker-controlled JS. CDP from outside the renderer is strictly better.

| Provider    | Migrated?     | What loads at startup            |
| ----------- | ------------- | -------------------------------- |
| WhatsApp    | ✅             | Zero JS                          |
| Telegram    | ✅             | Zero JS                          |
| Slack       | ✅             | Zero JS                          |
| Discord     | ✅             | Zero JS                          |
| browserscan | ✅             | Zero JS                          |
| Gmail       | grandfathered | Legacy `runtime.js` bridge       |
| LinkedIn    | grandfathered | Legacy `LINKEDIN_RECIPE_JS`      |
| Google Meet | grandfathered | Camera + audio + caption bridges |

Legacy injection should shrink, never grow. New providers go straight onto the CDP-only path.

## CEF prewarm

A hidden CEF webview (`cef-prewarm`) boots the browser on app launch so the first child webview spawns instantly when the user clicks. It's torn down before `cef::shutdown()` to avoid races during quit. See `app/src-tauri/src/lib.rs` around the prewarm + close lifecycle.

## Linux shell fallback for CEF startup crashes

On some Linux desktops, especially NVIDIA proprietary driver setups under Wayland/XWayland, the Tauri/CEF shell can fail during native window configuration before the React app becomes usable. One known symptom is an X11 `BadWindow` error after CEF reports the main browser context.

When the core itself is healthy, you can keep developing by running the core and frontend separately:

```bash
cargo build --bin openhuman-core
./target/debug/openhuman-core run --port 7788
```

In another terminal:

```bash
cd app
pnpm dev
```

Open the Vite URL in a regular browser, choose **Advanced** / remote core mode, set the RPC URL to `http://127.0.0.1:7788/rpc`, and use the bearer token written by the core. This bypasses native-only features such as tray, auto-update, and embedded provider webviews, but keeps the agent, memory, skills, and RPC surface available for debugging.

## Plugin audit

Anything new added to `app/src-tauri/src/lib.rs` must be audited for `js_init_script` calls. `tauri-plugin-opener` ships an init script (`init-iife.js`) by default that adds a global click listener; we configure it with `.open_js_links_on_click(false)` so it doesn't run inside third-party webviews. `tauri-plugin-notification`'s init script was likewise dropped from the vendored copy.

## Where this could evolve

The CDP surface is general-purpose. Today it powers memory ingest from a fixed list of providers; the same primitive can do much more.

### Browser automation as a first-class agent tool

Today the agent has [native tools](../features/native-tools/README.md) for filesystem, git, web search, and web fetch. The next obvious tool is **"drive a real browser session"**: log into a SaaS the user is already authed in, fill a form, scrape a paginated table, download an export.

The plumbing is already there. A `@openhuman/browser_task` skill could spin up a dedicated CEF webview, drive it via CDP from the core, and surface the result as a tool call. The user's existing per-account profiles mean no re-auth.

### Headless CEF for server-side replay

The same scanner pattern (long-lived WebSocket → IDB walk + DOM snapshot) works without a UI. Headless CEF in the core sidecar could replay sessions on a schedule, useful for users who host the core in the cloud and want auto-fetch from sources that don't expose a clean OAuth API.

### Privacy hooks at the browser-process layer

CEF's `CefRequestHandler` already lets us intercept network requests. A small step from "intercept and log" to "intercept and rewrite": ad-block, tracker-block, DNS pinning, request rewriting per provider. Privacy as a first-class browser feature instead of a leaky JS shim inside each origin.

### CDP-driven testing framework

The scanner pattern, spawn webview, walk IDB, snapshot DOM, evaluate one ephemeral expression, is structurally identical to E2E test orchestration. We could ship `@openhuman/web_test` as a public skill: `connect_cef → snapshot → evaluate → assert`. Tests written in plain Rust against any web app, no Selenium / Playwright dependency.

### Renderer ↔ Rust message channel

Today every CDP `Runtime.evaluate` is fire-and-forget. A long-lived bidirectional channel from renderer to Rust (the way Tauri does IPC for the host app) would unlock streaming use cases: live typing detection, real-time selection / highlight tracking, proactive nudges. Designing this so it doesn't violate the "no persistent JS bridge in third-party origins" rule is the interesting constraint.

### Multi-account merge

Each connected account gets its own profile and its own IDB. CDP can snapshot one account's IDB, decrypt-merge with another's, and upsert into a shared memory doc, e.g. one unified Slack memory across three workspaces.

## See also

* [`docs/TAURI_CEF_FINDINGS_AND_CHANGES.md`](../../docs/TAURI_CEF_FINDINGS_AND_CHANGES.md). the notification-permission deep dive.
* [`CLAUDE.md`](../../CLAUDE.md). the canonical "no new JS injection" rule.
