# Tauri CEF Notification Findings And Changes

## Scope

This note summarizes:

- what was found in `tauri-cef`
- what was missing for webview notification permission and delivery
- what was changed in `tauri-cef`
- what was changed in `openhuman-cursor`
- how the setup was debugged and verified

Relevant codebases:

- `/Users/megamind/tinyhuman/tauri-cef`
- `/Users/megamind/tinyhuman/openhuman-cursor`
- vendored submodule: `/Users/megamind/tinyhuman/openhuman-cursor/app/src-tauri/vendor/tauri-cef`

## Initial Findings In `tauri-cef`

### 1. Browser-side permission acceptance existed

`tauri-runtime-cef` already had browser-process logic that accepted Chromium notification permission requests:

- `crates/tauri-runtime-cef/src/permissions.rs`
- `crates/tauri-runtime-cef/src/cef_impl.rs`

This meant CEF could accept `CEF_PERMISSION_TYPE_NOTIFICATIONS`.

### 2. Renderer-side granted state was not wired into the real runtime path

Slack and similar apps do not rely only on the browser permission callback. They also inspect browser-visible JavaScript state:

- `Notification.permission`
- `Notification.requestPermission()`
- `navigator.permissions.query({ name: "notifications" })`

A renderer-side shim for this behavior existed only in:

- `cef-helper/src/notification.rs`

But the actual runtime app path used by `tauri-runtime-cef` did not attach that renderer process handler in the default `TauriApp` path. As a result, web apps could still observe notification state as `"prompt"` instead of `"granted"`.

### 3. Notification permission looked partially implemented, not end-to-end

There was a notification IPC path and registry in `tauri-runtime-cef`, but the setup was incomplete unless the embedder registered a handler:

- browser process received notification IPC
- runtime exposed `notification::register(...)`
- without an app-side registration, notifications could still be dropped

### 4. The old behavior was not sufficient for Slack

In practice, Slack kept behaving as if notifications still needed to be enabled because the renderer-visible granted state was not consistently exposed on the real runtime path.

### 5. Additional root cause found during live debugging

Later live DevTools inspection revealed a second, more concrete failure mode in `openhuman-cursor`:

- `tauri-plugin-notification` injects its own JavaScript init script into every Tauri webview
- that init script overwrote `window.Notification`
- the replacement implementation forwarded notification calls to:
  - `plugin:notification|notify`
  - over Tauri IPC at `http://ipc.localhost/...`

For external web pages such as Slack, this is the wrong path:

- the page is not supposed to use Tauri IPC as its notification transport
- the page should stay on the native CEF notification path
- when the plugin shim won, calls failed with `500` and console errors such as:
  - `POST http://ipc.localhost/plugin%3Anotification%7Cnotify 500`
  - `Origin header is not a valid URL`

That meant the plugin’s JS shim was effectively undoing the CEF notification interception fix inside external webviews.

## Changes Made In `tauri-cef`

These changes were made in the standalone `tauri-cef` repo and pushed there first.

### 1. Moved notification permission shims into the real runtime path

Renderer-side notification permission shims were added to `tauri-runtime-cef` so they run on the real Tauri CEF runtime path instead of only in the standalone helper.

Files involved:

- `/Users/megamind/tinyhuman/tauri-cef/crates/tauri-runtime-cef/src/notification.rs`
- `/Users/megamind/tinyhuman/tauri-cef/crates/tauri-runtime-cef/src/cef_impl.rs`
- `/Users/megamind/tinyhuman/tauri-cef/crates/tauri-runtime-cef/src/lib.rs`

Behavior provided by the shim:

- `Notification.permission` resolves to granted
- `Notification.requestPermission()` resolves to granted
- `navigator.permissions.query({ name: "notifications" })` reports granted
- notification calls are forwarded through the CEF runtime notification path

### 2. Hooked the render process handler into `TauriApp`

The runtime app path now installs the render process handler needed for the notification shim.

### 3. Started the helper process with a real app object

`run_cef_helper_process()` was updated to launch CEF with an app object instead of `None`, so the notification renderer setup is available consistently.

### 4. Added notification handler cleanup on browser close

In the vendored `tauri-cef` inside `openhuman-cursor`, an additional lifecycle cleanup was added:

- `/Users/megamind/tinyhuman/openhuman-cursor/app/src-tauri/vendor/tauri-cef/crates/tauri-runtime-cef/src/cef_impl.rs`

Added on browser close:

```rust
crate::notification::unregister(browser_id);
```

This prevents stale notification handlers from remaining registered after a webview is destroyed.

### 5. Standalone `tauri-cef` commit

The standalone `tauri-cef` repo was updated and pushed with:

- branch: `feat/cef`
- commit: `c8ece7c78`
- message: `Fix CEF notification permission shims`

The `openhuman-cursor` vendored submodule was then updated to the corresponding submodule commit:

- `c8ece7c784b8cdff16dc552f6892a0c9982ef1ba`

## Changes Made In `openhuman-cursor`

### 1. Enabled shell-side webview notifications by default

File:

- `/Users/megamind/tinyhuman/openhuman-cursor/app/src-tauri/src/notification_settings/mod.rs`

Change:

- default notification toggle changed to enabled:

```rust
AtomicBool::new(true)
```

This ensures notifications are not disabled by default at the app layer.

### 2. Added a Slack debugging bypass for internal CDP attachment

File:

- `/Users/megamind/tinyhuman/openhuman-cursor/app/src-tauri/src/webview_accounts/mod.rs`

Environment flag added:

- `OPENHUMAN_DISABLE_SLACK_SCANNER=1`

When set for Slack accounts, the app now:

- skips Slack scanner startup
- skips the long-lived CDP session for Slack
- loads the real Slack URL directly instead of going through the placeholder `data:` path used for the CDP bootstrap flow

Expected logs:

- `[webview-accounts] skipping CDP session via OPENHUMAN_DISABLE_SLACK_SCANNER ...`
- `[webview-accounts] slack scanner disabled via OPENHUMAN_DISABLE_SLACK_SCANNER ...`

This was added only to make manual DevTools inspection possible without the app attaching to the same Slack target.

### 3. Vendored `tauri-plugin-notification` and removed its JS init script

To stop `window.Notification` from being overwritten inside external webviews, `tauri-plugin-notification` was vendored into the repo and switched from a git dependency to a path dependency.

New vendored path:

- `/Users/megamind/tinyhuman/openhuman-cursor/app/src-tauri/vendor/tauri-plugin-notification`

Two changes were made:

1. The plugin dependency in:
   - `/Users/megamind/tinyhuman/openhuman-cursor/app/src-tauri/Cargo.toml`
   now points to the vendored path.

2. The plugin init function in:
   - `/Users/megamind/tinyhuman/openhuman-cursor/app/src-tauri/vendor/tauri-plugin-notification/src/lib.rs`
   no longer calls:

```rust
.js_init_script(...)
```

This keeps the Rust-side desktop notification API available through `NotificationExt`, but stops the plugin from globally replacing `window.Notification` in embedded external pages.

The result is:

- app Rust code can still fire native notifications
- external webviews like Slack no longer get forced onto the Tauri IPC notification path
- the native CEF notification shim remains authoritative inside the external webview

## Verification Performed

### Build verification

In `/Users/megamind/tinyhuman/openhuman-cursor/app/src-tauri`:

- `cargo fmt` passed
- `cargo check --features cef` passed for the main notification changes
- `cargo check --features cef` also passed after vendoring `tauri-plugin-notification` and removing its JS init script

One later `cargo check` attempt for the Slack DevTools bypass work was blocked by another running Cargo process holding the build lock, but the changes were localized and formatting completed.

### Runtime verification

The app was run in dev mode and the CEF DevTools target list was checked through:

- `http://localhost:9222/json/list`
- `http://127.0.0.1:9222/json/list`
- earlier in one run, `http://[::1]:9222/json/list`

Observed targets included:

- Slack page target
- Slack service worker target
- OpenHuman page target

This confirmed:

- Slack was running inside the CEF webview
- CEF remote debugging was active

### Slack scanner contention diagnosis

At first, the main reason DevTools inspection failed for Slack was that the app itself was attaching to the Slack target over CDP:

- the Slack scanner auto-attached
- the long-lived per-account CDP session also attached

This was why manual DevTools sessions disconnected even when the target existed.

The debugging bypass confirmed this by producing logs that both internal attachment paths were skipped.

## Final DevTools Findings

After the Slack-specific CDP paths were disabled, DevTools could still disconnect even for the `OpenHuman` page target. That showed the remaining issue was not Slack-specific.

Live verification showed:

- the active backend was on `127.0.0.1:9222`
- `localhost` was inconsistent during checks
- Brave already had multiple established connections to `127.0.0.1:9222`
- the running `OpenHuman` app was listening on that port

Most likely explanation:

- multiple existing DevTools frontend sessions in Brave were already connected
- reopening new inspector tabs caused connection churn
- `127.0.0.1` was more reliable than `localhost`

Recommended DevTools usage:

1. Close all existing DevTools tabs for `localhost:9222` and `127.0.0.1:9222`.
2. Reopen only one inspector at a time.
3. Prefer the exact `127.0.0.1` websocket host advertised by `/json/list`.

## Later Runtime And Helper Findings

### 1. The real helper bundle was stale at first

During live verification, the main app binary contained the new notification instrumentation but the bundled macOS helper apps did not.

That turned out to be a packaging issue:

- the installed `cargo-tauri` binary was stale
- helper executables are bundled by the CLI/bundler
- rebuilding the app alone was not enough to refresh the helper binaries

To prevent this from recurring, the local CLI bootstrap script was updated:

- `/Users/megamind/tinyhuman/openhuman-cursor/scripts/ensure-tauri-cli.sh`

It now reinstalls vendored `cargo-tauri` when vendored `tauri-cef` sources are newer than the installed CLI binary.

### 2. The live macOS renderer path uses `cef-helper`

An important debugging detail was that the actual live renderer helper on macOS was using:

- `/Users/megamind/tinyhuman/openhuman-cursor/app/src-tauri/vendor/tauri-cef/cef-helper/src/notification.rs`

and not only the runtime-side notification file in:

- `/Users/megamind/tinyhuman/openhuman-cursor/app/src-tauri/vendor/tauri-cef/crates/tauri-runtime-cef/src/notification.rs`

So the helper-side shim was instrumented directly with:

- install logs
- execution logs
- debug markers
- manual test entry points

Helper-side logs added:

- `[cef-helper-notify] on_context_created ...`
- `[cef-helper-notify] installed shims ...`
- `[cef-helper-notify] execute source=...`

Helper-side debug markers added:

- `window.__OPENHUMAN_CEF_NOTIFICATION_SHIM`
- `window.__OPENHUMAN_CEF_NOTIFICATION_ORIGIN`
- `Notification.__openhuman_cef`

Manual helper entry points added:

- `window.__openhumanFireNotification(title, options)`
- `window.__OPENHUMAN_CEF_NOTIFICATION_CONSTRUCTOR`

### 3. Helper shim installation was verified in the real Slack page

After rebuilding the helper bundle, live DevTools checks showed:

- `window.__OPENHUMAN_CEF_NOTIFICATION_SHIM === true`
- `window.__OPENHUMAN_CEF_NOTIFICATION_ORIGIN` pointing at the Slack page URL

This proved that the CEF helper render shim was actually installing in the Slack page context.

### 4. Manual helper-triggered notifications work end-to-end

The following manual trigger was verified to reach the app:

```js
window.__openhumanFireNotification("Slack CEF test", {
  body: "Manual helper path"
})
```

Observed runtime logs:

- `[cef-notify] ipc ...`
- `[cef-notify] dispatch ...`
- `[notify-cef][...] ...`

This proved that:

- renderer helper -> browser IPC works
- runtime dispatch works
- `openhuman-cursor` receives the notification payload
- OS notification delivery can work

### 5. Tokio runtime panic found and fixed in app notification delivery

Once the manual helper path reached the app, native notification delivery initially crashed with:

- `there is no reactor running, must be called from the context of a Tokio 1.x runtime`

The panic came from using `tokio::task::spawn_blocking(...)` from a CEF callback thread in:

- `/Users/megamind/tinyhuman/openhuman-cursor/app/src-tauri/src/webview_accounts/mod.rs`

Fix applied:

- replaced `tokio::task::spawn_blocking(...)` with `std::thread::spawn(...)`

This was done for the macOS notification path and the Linux path for consistency.

After this fix, the manual helper trigger produced an actual OS notification successfully.

### 6. `Invalid UTF-16 string` messages were observed but were not the blocker

During manual notification tests, CEF emitted:

- `Invalid UTF-16 string`

These warnings appeared before successful notification IPC and dispatch logs, so they were treated as noisy string-conversion warnings rather than the root failure.

### 7. Slack still does not automatically use the browser notification APIs for real messages

After helper-side execution logging was added, a real incoming Slack message did **not** produce:

- `[cef-helper-notify] execute source=0 ...`
- `[cef-helper-notify] execute source=1 ...`

This means the real incoming-message path in this embedded Slack session is not currently hitting:

- `new Notification(...)`
- `ServiceWorkerRegistration.prototype.showNotification(...)`

So the remaining problem is no longer CEF notification transport. The remaining problem is Slack-specific runtime behavior in this embedded environment.

### 8. An attempted hard lock on `window.Notification` broke Slack rendering

One experiment tried to prevent Slack from overwriting the helper-installed notification hooks by making them effectively non-overridable.

That caused Slack to render a blank screen, so that hardening change was reverted.

Current safe state:

- Slack renders normally
- helper shim installs
- manual helper trigger works
- automatic incoming-message notifications still do not use the browser notification APIs

### 9. Fallback strategy: synthesize notifications from the Slack scanner

Because Slack’s own incoming-message path was not invoking browser notification APIs, a fallback path was added to the Slack scanner:

- `/Users/megamind/tinyhuman/openhuman-cursor/app/src-tauri/src/slack_scanner/mod.rs`
- `/Users/megamind/tinyhuman/openhuman-cursor/app/src-tauri/src/webview_accounts/mod.rs`

The scanner logic was updated to:

- keep a per-channel unread baseline
- skip notifications on the first scan
- emit a native notification when a channel unread count increases later

However, live verification showed the scanner path was not actually active because the app was missing the scanner registry in managed state.

Observed log:

- `[webview-accounts] slack ScannerRegistry not in app state`

So the scanner-based fallback is patched in code, but it still needs the app builder to manage `slack_scanner::ScannerRegistry::new()` in:

- `/Users/megamind/tinyhuman/openhuman-cursor/app/src-tauri/src/lib.rs`

## Current Outcome

### Notification permission behavior

The underlying notification permission problem in `tauri-cef` was fixed by moving the renderer-visible granted-state shim into the actual runtime path.

This means Slack-style checks should now see notification permission as granted inside the Tauri CEF webview.

### App-side notification behavior

`openhuman-cursor` was updated so:

- shell-side notifications are enabled by default
- the vendored `tauri-cef` includes the runtime permission fix
- browser notification handlers are cleaned up on webview close
- manual helper-triggered notifications reach the OS successfully
- app-side native notification delivery no longer depends on a Tokio runtime in the callback thread

### Remaining distinction

There are two separate concerns:

1. Permission/granted-state correctness
2. What the app does after a notification is received

The changes above fix the permission side and the runtime notification bridge plumbing. The app still needs its normal notification handling path to decide how to present or forward those notifications.

### Current verified status

Verified working:

- notification permission appears granted in the Slack webview
- helper shim installs in the live Slack page
- manual helper notification trigger reaches CEF browser IPC
- runtime dispatch reaches `openhuman-cursor`
- native OS notification display works for the manual helper-triggered path

Still not working automatically:

- real Slack incoming messages do not currently surface through browser notification APIs in this embedded session
- scanner-driven fallback notifications will not run until the scanner registry is re-enabled in app state

## Files Changed

### In `tauri-cef`

- `/Users/megamind/tinyhuman/tauri-cef/crates/tauri-runtime-cef/src/notification.rs`
- `/Users/megamind/tinyhuman/tauri-cef/crates/tauri-runtime-cef/src/cef_impl.rs`
- `/Users/megamind/tinyhuman/tauri-cef/crates/tauri-runtime-cef/src/lib.rs`
- `/Users/megamind/tinyhuman/tauri-cef/CEF_NOTIFICATION_PERMISSION_CHANGES.md`

### In `openhuman-cursor`

- `/Users/megamind/tinyhuman/openhuman-cursor/app/src-tauri/src/notification_settings/mod.rs`
- `/Users/megamind/tinyhuman/openhuman-cursor/app/src-tauri/src/webview_accounts/mod.rs`
- `/Users/megamind/tinyhuman/openhuman-cursor/app/src-tauri/src/slack_scanner/mod.rs`
- `/Users/megamind/tinyhuman/openhuman-cursor/app/src-tauri/src/lib.rs`
- `/Users/megamind/tinyhuman/openhuman-cursor/app/src-tauri/Cargo.toml`
- `/Users/megamind/tinyhuman/openhuman-cursor/app/src-tauri/vendor/tauri-plugin-notification/Cargo.toml`
- `/Users/megamind/tinyhuman/openhuman-cursor/app/src-tauri/vendor/tauri-plugin-notification/src/lib.rs`
- `/Users/megamind/tinyhuman/openhuman-cursor/app/src-tauri/vendor/tauri-cef/crates/tauri-runtime-cef/src/cef_impl.rs`
- `/Users/megamind/tinyhuman/openhuman-cursor/app/src-tauri/vendor/tauri-cef/crates/tauri-runtime-cef/src/notification.rs`
- `/Users/megamind/tinyhuman/openhuman-cursor/app/src-tauri/vendor/tauri-cef/crates/tauri-runtime-cef/src/lib.rs`
- `/Users/megamind/tinyhuman/openhuman-cursor/app/src-tauri/vendor/tauri-cef/cef-helper/src/notification.rs`
- `/Users/megamind/tinyhuman/openhuman-cursor/app/src-tauri/vendor/tauri-cef/cef-helper/src/main.rs`
- `/Users/megamind/tinyhuman/openhuman-cursor/scripts/ensure-tauri-cli.sh`

## Suggested Follow-Up

Recommended next functional fix:

1. Re-enable `slack_scanner::ScannerRegistry::new()` in:
   - `/Users/megamind/tinyhuman/openhuman-cursor/app/src-tauri/src/lib.rs`
2. Rebuild and verify unread-delta notifications from the scanner fallback path.

Secondary cleanup work:

1. Remove temporary helper/debug instrumentation once notification behavior is finalized.
2. If cleaner inspection is still needed, move remote debugging off `9222` to a dedicated port such as `9333`.
