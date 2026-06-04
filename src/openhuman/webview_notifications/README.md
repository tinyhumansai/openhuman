# webview_notifications

Routes `window.Notification` calls fired inside embedded provider webviews (Slack, Gmail, Discord, …) to native OS toasts. The actual CEF IPC capture and OS toast firing live in the Tauri shell (`app/src-tauri/`); this core domain owns only the **shared wire types**, the **title-formatting contract** (the `OpenHuman:` prefix used to dedup against locally-installed native apps), and **placeholder controller stubs** for a future JSON-RPC on/off toggle. v1 is intentionally thin plumbing — the feature ships disabled by default and most behavior runs shell-side.

## Responsibilities

- Define the `WebviewNotificationEvent` wire payload carried from the Tauri shell to the React UI (over the `webview-notification:fired` Tauri event).
- Define the `NotificationSettings` on/off toggle type (defaults to **disabled**).
- Own `format_title` / `OPENHUMAN_TITLE_PREFIX` — the canonical `OpenHuman:`-prefixed native-toast title format, shared so the shell and core agree byte-for-byte.
- Participate in the controller registry (`src/core/all.rs`) via empty schema/controller stubs so future additions (notification history, per-account mute) are a trivial extend.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/webview_notifications/mod.rs` | Module docstring + `pub mod` decls + `pub use` re-exports. Export-focused, no logic. |
| `src/openhuman/webview_notifications/types.rs` | Serde/JsonSchema wire types: `WebviewNotificationEvent`, `NotificationSettings` (default disabled). |
| `src/openhuman/webview_notifications/dispatch.rs` | `format_title()` + `OPENHUMAN_TITLE_PREFIX` const; title-formatting contract shared with the Tauri shell. Holds the module's unit tests. |
| `src/openhuman/webview_notifications/schemas.rs` | Controller registry stubs — both `all_*` fns return empty `Vec`s (no RPC surface yet). |
| `src/openhuman/webview_notifications/bus.rs` | Docstring-only placeholder; no `EventHandler` impls in v1. |

## Public surface

From `mod.rs` re-exports:

- `dispatch::format_title` — `format_title(provider_label, raw_title) -> String`. Produces `OpenHuman: <Provider> — <raw title>`, collapsing to `OpenHuman: <Provider>` when the raw title is empty/whitespace.
- `dispatch::OPENHUMAN_TITLE_PREFIX` — `"OpenHuman: "` (trailing space).
- `types::WebviewNotificationEvent` — `{ account_id, provider, title, body, tag? }`. `tag` is skipped when `None`.
- `types::NotificationSettings` — `{ enabled: bool }`, `Default` = `false`.
- `schemas::all_webview_notifications_controller_schemas`, `schemas::all_webview_notifications_registered_controllers` — registry hooks (currently empty).

## RPC / controllers

None in v1. `all_webview_notifications_controller_schemas()` and `all_webview_notifications_registered_controllers()` both return empty vectors. They are still wired into `src/core/all.rs` so the domain participates in the registry like every other domain; the user-facing on/off toggle currently lives in the Tauri shell as per-install state rather than core config.

## Agent tools

None — no `tools.rs`.

## Events

None published or subscribed in v1. `bus.rs` is a documented placeholder. The shell fires notifications directly to the frontend over the Tauri event bus (`webview-notification:fired`); core subscribers (e.g. archiving notification history into memory) would land in `bus.rs` as `EventHandler` impls when needed.

## Persistence

None — no `store.rs`. The on/off toggle is per-install state owned by the Tauri shell, not persisted by this domain.

## Dependencies

- `crate::core::all::RegisteredController` — registry type returned by the controller stub.
- `crate::core::ControllerSchema` — schema type returned by the controller stub.

No `crate::openhuman::*` dependencies. External crates: `serde`, `schemars` (for the wire types).

## Used by

- `src/openhuman/mod.rs` — declares the module (`pub mod webview_notifications;`).
- `src/core/all.rs` — calls both `all_*` registry functions (controllers + schemas).
- `app/src-tauri/src/webview_accounts/mod.rs` — the Tauri shell's title-prefix logic is documented to match `OPENHUMAN_TITLE_PREFIX` (cross-crate contract; comment reference, not a code import).

## Notes / gotchas

- **Disabled by default**: `NotificationSettings::default()` is `enabled: false` so the release doesn't suddenly fire OS toasts for every background DM in an idle webview tab. v1 ships the plumbing only.
- **The interesting code is in the Tauri shell, not here.** The CEF IPC hook that captures the renderer-side `window.Notification` call and fires the native toast lives in the `openhuman` crate at `app/src-tauri/` (`tauri_runtime_cef::notification::register`). This core domain is deliberately minimal.
- **The `OpenHuman:` prefix is a dedup mechanism**, not cosmetic: embedded webviews can run alongside the user's locally-installed native app for the same service, and both fire toasts for the same event — the prefix lets the user (and the OS notification centre grouping) distinguish OpenHuman's toast.
- The shell and core must agree on the title format; keep `format_title` / `OPENHUMAN_TITLE_PREFIX` in sync with the shell's matching logic.
