# Issue #714 — Native OS Notifications from Embedded Webviews

**Branch**: `feat/714-native-os-notifications`
**Base**: `upstream/main`
**Upstream**: `tinyhumansai/openhuman`
**Origin (fork)**: `oxoxDev/openhuman`

---

## Problem

Embedded webview apps (Slack, Discord, Gmail, WhatsApp) call `window.Notification` inside CEF but never produce native macOS/Windows toasts. The CEF runtime intercepts the web Notification API, but the intercept dropped on the floor — no bridge to `tauri-plugin-notification`, no click routing back to the originating account, no permission query/request pipeline.

## Solution

Wire the `tauri-cef` notification intercept into `tauri-plugin-notification`, prefix each toast with the provider label (e.g. `[Slack] New message from Alice`), honour `silent` / `icon` / `tag`, and record a `NotificationRoute` keyed by `{provider}:{account_id}:{tag_or_uuid}` so a future platform click hook can emit `notification:click` and route focus back to the correct account. Also round-trip the OS notification permission via new invokes so the frontend sees the same `"granted" | "denied" | "default"` triple as the web API on both CEF and wry runtimes.

## Commits (in order)

### `50b831ad` feat(webview_accounts): native OS notifications from embedded webviews (#714)
Rust backend — the core of the feature.

- **`app/src-tauri/src/webview_accounts/mod.rs`** (+141 / -3)
  - `NotificationRoute` struct: `provider`, `account_id`, `tag`, `created_at`
  - `notification_routes: Mutex<HashMap<String, NotificationRoute>>` on `WebviewAccountState`
  - `clear_notification_routes(account_id)` — purged on close / purge
  - `forward_native_notification(app, provider, account_id, payload)`:
    - Prefixes title with `[Provider]`
    - Respects `silent` (records route, skips toast)
    - Passes `icon` through to builder
    - Uses `tag` as dedup key, falls back to monotonic timestamp
  - `tag_or_uuid` helper — tag is the web API's dedup key; timestamp fallback ensures untagged payloads route uniquely
  - `webview_notification_permission_state` / `_request` — map `tauri::plugin::PermissionState` (`Granted | Denied | Prompt | PromptWithRationale`) onto `"granted" | "denied" | "default"`
  - `permission_state_str` helper
  - Non-cef (wry) stubs return `"default"` so frontend calls same invoke names on both runtimes
  - CEF registration in `setup`: `tauri_runtime_cef::notification::register` with handler that calls `forward_native_notification`; `unregister` on account close

- **`app/src-tauri/src/lib.rs`** (+2)
  - Added `webview_notification_permission_state` and `webview_notification_permission_request` to the invoke handler list.

- **`app/src-tauri/capabilities/default.json`** (+3)
  - Added `notification:allow-notify`, `notification:allow-request-permission`, `notification:allow-is-permission-granted` so the plugin can be invoked from the webview context.

### `97ef390f` feat(accounts): wire notification permission + click bridge (#714)
Frontend — permission round-trip + dormant click listener.

- **`app/src/services/webviewAccountService.ts`** (+59 / -1)
  - `ensureNotificationPermission(accountId)` — invokes `webview_notification_permission_state`, requests if `"default"`, runs once per session on first account open. Desktop plugin auto-grants today, but shape matches web API so future platform prompts slot in without UI change.
  - `handleNotificationClick` + `listen('notification:click', …)` — dispatches `setActiveAccount` and invokes `activate_main_window` when the (currently dormant) platform click hook emits the event. Contract matches Rust `NotificationRoute` shape so Rust emit side is a one-liner when UNUserNotificationCenter / notify-rust `on_response` is wired.
  - `openWebviewAccount` now calls `void ensureNotificationPermission(accountId)` after the account opens.

### `e6f60180` chore: sync Cargo.lock to 0.52.26 after version bump
- **`Cargo.lock`** + **`app/src-tauri/Cargo.lock`** (+2 / -2 each)
  - Picked up pending 0.52.26 version bump while building. No dependency graph change.

---

## Quality Gates (all passed)

| Gate | Result | Time |
|---|---|---|
| `pnpm compile` (tsc --noEmit) | pass | 32.30s |
| `pnpm lint` (eslint) | pass | 63.65s |
| `pnpm rust:format:check` | pass | — |
| `cargo check --features cef --no-default-features` | pass | 22.21s |
| `cargo check --features wry --no-default-features` | pass | 6m 29s (cold) |

**Skipped:**
- `pnpm format:check` — flags only `app/src/pages/Home.tsx` (local build-tag pill `#714`, `skip-worktree` flagged, per workflow Phase 3 Step 6). Confirmed via `git ls-files -v | grep '^S '` → `S app/src/pages/Home.tsx`.
- `cargo clippy` — pre-existing errors in `src/slack_scanner/extract.rs` (type_complexity) and `src/lib.rs:212` (unnecessary_map_or) unrelated to this feature. Verified with `git diff upstream/main -- app/src-tauri/src/lib.rs` shows only the 2-line invoke handler addition.

**Not yet done:**
- Manual verification in built `.app` bundle with real Slack/Discord/Gmail notifications. Requires `pnpm macOS:build:debug` (~10 min), install, open, trigger notifications, confirm provider-prefixed titles fire natively.

---

## Key Files for Teammate Review

| File | Role |
|---|---|
| `app/src-tauri/src/webview_accounts/mod.rs` | Core Rust logic — intercept handler, route table, permission commands |
| `app/src-tauri/src/lib.rs` | Invoke handler registration |
| `app/src-tauri/capabilities/default.json` | Notification plugin capabilities |
| `app/src/services/webviewAccountService.ts` | Frontend permission round-trip + click bridge |
| `app/src-tauri/vendor/tauri-cef/crates/tauri-runtime-cef/src/notification.rs` | (vendored, unchanged) — source of the `register`/`unregister`/`dispatch` API used here |

---

## Architecture Notes

### Route keying
`{provider}:{account_id}:{tag_or_uuid}` — tag is the web Notifications API dedup key (second `new Notification(title, { tag })` with same tag replaces the first). When absent, fall back to `Instant::now()` monotonic timestamp so every untagged payload routes uniquely. This matches browser semantics and prevents map collisions when two accounts of the same provider fire untagged notifications simultaneously.

### Permission shape
`tauri::plugin::PermissionState` has 4 variants but the web API only has 3. Map:
- `Granted` → `"granted"`
- `Denied` → `"denied"`
- `Prompt`, `PromptWithRationale` → `"default"`

Non-cef runtime stubs always return `"default"` — prevents invoke name mismatch between runtimes so frontend doesn't need a feature flag.

### Dormant click listener
`notification:click` listener is registered frontend-side but Rust doesn't emit it yet. UNUserNotificationCenter (macOS) and notify-rust `on_response` (Linux/Windows) callbacks are the platform hooks that will emit once wired. The route table is already populated by the notification dispatch path so the emit side is a one-liner:

```rust
let route = state.notification_routes.lock().unwrap().get(&route_key).cloned();
if let Some(r) = route {
    app.emit("notification:click", &r)?;
}
```

---

## Next Steps for Teammate

1. **Manual verification** — build `.app`, test Slack/Discord/Gmail toasts, confirm title prefix, confirm `silent` / `icon` / `tag` all honoured.
2. **Platform click hooks** — wire UNUserNotificationCenter delegate (macOS) and notify-rust `on_response` (Linux/Windows) to emit `notification:click` with the stored `NotificationRoute`. Route table already exists; emit is one line.
3. **PR** — template headings required: `## Summary`, `## Problem`, `## Solution`, `## Submission Checklist`, `## Impact`, `## Related`. `Closes #714`.

---

## Local State Caveats

- **Home.tsx build-tag pill** — `skip-worktree` flag set on `app/src/pages/Home.tsx` with inline `#714` pill (top-right, fixed). Per-clone, does NOT travel with branch. If teammate pulls this branch into their own clone, no pill appears locally. If they want one, Phase 3 Step 6 of `.claude/rules/00-workflow.md` has the snippet.
- **Cargo.lock** — version bumped to 0.52.26 locally. Separate commit `e6f60180` so diff review is clean.
