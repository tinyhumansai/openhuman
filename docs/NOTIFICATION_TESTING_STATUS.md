# Native OS Notification — Testing Status

Companion to `TAURI_CEF_FINDINGS_AND_CHANGES.md`.  
This file is a quick-reference checklist: what is done, what is still needed, and how to test.

---

## What Is Done

### tauri-cef (vendored submodule)

| Change | File |
|--------|------|
| `NotifyRenderProcessHandler` wired into `TauriApp::render_process_handler` | `vendor/tauri-cef/crates/tauri-runtime-cef/src/cef_impl.rs` |
| `run_cef_helper_process()` uses `NotifyApp` (not `None`) | `vendor/tauri-cef/crates/tauri-runtime-cef/src/lib.rs` |
| `notification::unregister(browser_id)` called on browser close | `vendor/tauri-cef/crates/tauri-runtime-cef/src/cef_impl.rs` |
| Dispatch logs added (`[cef-notify] dispatch` / dropped) | `vendor/tauri-cef/crates/tauri-runtime-cef/src/notification.rs` |
| `on_context_created` install logs in runtime shim | `vendor/tauri-cef/crates/tauri-runtime-cef/src/notification.rs` |
| `on_context_created` install logs in helper shim | `vendor/tauri-cef/cef-helper/src/notification.rs` |
| Debug markers: `window.__OPENHUMAN_CEF_NOTIFICATION_SHIM`, `__OPENHUMAN_CEF_NOTIFICATION_ORIGIN` | `vendor/tauri-cef/cef-helper/src/notification.rs` |
| Manual test entry point: `window.__openhumanFireNotification(title, opts)` | `vendor/tauri-cef/cef-helper/src/notification.rs` |
| `ensure-tauri-cli.sh` reinstalls vendored CLI when tauri-cef sources are newer | `scripts/ensure-tauri-cli.sh` |

### tauri-plugin-notification (vendored to stop init-script conflict)

| Change | File |
|--------|------|
| Plugin vendored at `vendor/tauri-plugin-notification/` | `app/src-tauri/Cargo.toml` |
| Plugin dependency switched from git rev to local path | `app/src-tauri/Cargo.toml` |
| `.js_init_script(...)` call removed from plugin `init()` | `vendor/tauri-plugin-notification/src/lib.rs` |

**Why this mattered:** Without this change, the plugin injected a JS shim that forwarded `new Notification(...)` to `http://ipc.localhost/plugin:notification|notify`. That IPC always fails with 500 in third-party webviews (Slack), overwriting the CEF shim and blocking all notification delivery.

### openhuman-cursor app shell

| Change | File |
|--------|------|
| Default notification toggle set to `true` | `src/notification_settings/mod.rs` |
| `OPENHUMAN_DISABLE_SLACK_SCANNER=1` env bypass for DevTools inspection | `src/webview_accounts/mod.rs` |
| Platform-specific OS notification with click detection added | `src/webview_accounts/mod.rs` |
| macOS: `mac-notification-sys` + `wait_for_click` + `std::thread::spawn` | `src/webview_accounts/mod.rs` |
| Linux: `notify-rust` + `wait_for_action` + `std::thread::spawn` | `src/webview_accounts/mod.rs` |
| Windows: fire-and-forget fallback via `NotificationExt` | `src/webview_accounts/mod.rs` |
| `notification:click` Tauri event emitted with `{ account_id, provider }` | `src/webview_accounts/mod.rs` |
| `[notify-click]` success logs promoted from `debug` to `info` | `src/webview_accounts/mod.rs` |
| `mac-notification-sys = "0.6"` added to macOS dependencies | `app/src-tauri/Cargo.toml` |
| `notify-rust` added to Linux dependencies | `app/src-tauri/Cargo.toml` |
| `NotificationExt` import scoped to `#[cfg(all(feature = "cef", windows))]` | `src/webview_accounts/mod.rs` |
| `tokio::task::spawn_blocking` replaced with `std::thread::spawn` (fixes tokio panic from CEF callback thread) | `src/webview_accounts/mod.rs` |
| Scanner fallback: per-channel unread baseline, delta-based notification synthesis | `src/slack_scanner/mod.rs` |

---

## What Is Still Needed

### 1. Re-enable the Slack scanner registry (BLOCKER for automatic notifications)

**File:** `app/src-tauri/src/lib.rs`

The scanner-driven fallback notification path is fully coded but never runs because `ScannerRegistry` is not registered in the Tauri app state. The log confirms this every time:

```
[webview-accounts] slack ScannerRegistry not in app state
```

**Fix:** In `lib.rs`, inside `tauri::Builder::default()...manage(...)`, add:

```rust
.manage(Arc::new(slack_scanner::ScannerRegistry::new()))
```

After this change:
- The scanner will track per-channel unread counts
- When a channel's unread count increases, the scanner synthesizes a native OS notification
- This is the fallback path because Slack's embedded session does not call `new Notification(...)` for real incoming messages

### 2. Verify end-to-end with a real incoming Slack message

Once the scanner registry is registered:

1. Run the app: `cd app && yarn dev:app`
2. Open the Slack webview — wait for Slack to load fully
3. Have someone send you a Slack message from another device
4. Watch the log:
   ```bash
   tail -f /tmp/openhuman-dev-app.log | grep --line-buffered "notify-cef\|notify-click\|scanner\|unread"
   ```
5. Expected log sequence:
   ```
   [scanner] unread delta channel=... prev=N new=M
   [notify-cef][<account_id>] source=... tag=... title_chars=N body_chars=M
   ```
6. OS toast should appear
7. Click the toast → expected:
   ```
   [notify-click][<account_id>] clicked provider=slack
   ```
8. Slack webview should come into focus (frontend routes `notification:click` → `setActiveAccount` → `activate_main_window`)

### 3. Verify the CEF shim installs in Slack's page context

Before relying on real messages, confirm the helper shim is active via DevTools:

1. Open `brave://inspect`
2. Find the Slack page target → click **Inspect**
3. In Console, run:
   ```js
   window.__OPENHUMAN_CEF_NOTIFICATION_SHIM   // should be true
   window.__OPENHUMAN_CEF_NOTIFICATION_ORIGIN  // should be the Slack URL
   ```
4. If both are present, the CEF helper shim installed correctly

### 4. Verify the manual helper trigger path end-to-end

With DevTools open on the Slack target:

```js
window.__openhumanFireNotification("Slack CEF test", { body: "Manual trigger" })
```

Expected log:
```
[cef-notify] dispatch browser_id=N source=Window title="Slack CEF test" origin=...
[notify-cef][<account_id>] source=Window tag= silent=false title_chars=14 body_chars=13
```

And an OS notification toast should appear. If no toast appears, the blocker is in `forward_native_notification` in `webview_accounts/mod.rs`.

### 5. Clean up debug instrumentation (post-verification)

Once notifications are working reliably, remove:

- `window.__OPENHUMAN_CEF_NOTIFICATION_SHIM` global marker
- `window.__OPENHUMAN_CEF_NOTIFICATION_ORIGIN` global marker
- `window.__openhumanFireNotification` manual trigger
- `window.__OPENHUMAN_CEF_NOTIFICATION_CONSTRUCTOR` saved reference
- `[cef-helper-notify]` `eprintln!` calls in `cef-helper/src/notification.rs` (or replace with proper `log::debug!`)

---

## How To Run The App Correctly

The app **must** be started with `dev:app`, not `tauri dev` directly:

```bash
cd app && yarn dev:app
```

`dev:app` sets `CEF_PATH=$HOME/Library/Caches/tauri-cef` and ensures the vendored `cargo-tauri` is installed. Without it, the app panics at startup in `cef::library_loader` with `No such file or directory`.

Live log location:

```bash
tail -f /tmp/openhuman-dev-app.log | grep --line-buffered "notify-cef\|notify-click\|scanner\|unread\|cef-notify"
```

---

## Key Log Prefixes

| Prefix | Where | Meaning |
|--------|-------|---------|
| `[cef-helper-notify] on_context_created` | renderer subprocess | shim callback fired for a new JS context |
| `[cef-helper-notify] installed shims` | renderer subprocess | shim installed in that context |
| `[cef-helper-notify] execute` | renderer subprocess | `new Notification(...)` called by page JS |
| `[cef-notify] dispatch` | browser process | notification IPC received from renderer, handler called |
| `[cef-notify] dropped` | browser process | notification IPC received but no handler registered |
| `[notify-cef][id]` | `webview_accounts` | notification payload reached app, OS toast being sent |
| `[notify-click][id] clicked` | `webview_accounts` | user clicked OS toast, emitting `notification:click` |
| `[webview-accounts] slack ScannerRegistry not in app state` | `webview_accounts` | scanner registry is missing — add `.manage(ScannerRegistry::new())` in `lib.rs` |

---

## Frontend Side (Already Wired, No Changes Needed)

`app/src/services/webviewAccountService.ts` already listens for `notification:click`:

```ts
listen('notification:click', ({ payload }) => {
  dispatch(setActiveAccount(payload.account_id));
  invoke('activate_main_window');
});
```

No frontend changes are needed. The click routing will work once the Rust side emits the event.
