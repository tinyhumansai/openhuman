# native_notifications

Tauri commands for OS notifications: read the permission state, prompt for
permission, and show a banner. The frontend uses them for the "Send test
notification" button and for surfacing agent and system events while the
window is not focused.

## How it works

The module exists because `tauri-plugin-notification` is not truthful on
macOS. Its desktop `permission_state()` and `request_permission()` always
report `Granted`, and its `.show()` runs the delivery on a background task
and drops the result. A permission gate built on the plugin passes even when
macOS has notifications turned off for the app, and the UI reports "sent"
for a banner that never appears.

On macOS the module calls `UNUserNotificationCenter` directly through `objc2`
and waits on each completion handler with a short timeout:

```text
show_native_notification(title, body, tag?)
   |
   +-- macOS --------------------------------------------------------+
   |   ensure_bundled_app()      reject an unbundled dev process     |
   |   permission_state()        getNotificationSettings... (2 s)    |
   |   not granted/provisional/ephemeral -> Err                      |
   |   UNMutableNotificationContent (title, body, default sound)     |
   |   identifier = tag or "openhuman.notify.<nanos>"                |
   |   addNotificationRequest...  completion -> Ok / Err(msg) (2 s)  |
   +-----------------------------------------------------------------+
   |
   +-- Windows / Linux ----------------------------------------------+
       app.notification().builder().title(..).body(..).show()        |
   ------------------------------------------------------------------+
```

`request_permission` asks for alert, badge and sound and waits up to 5
seconds for the user's answer. A unique identifier per call matters because
the notification center de-duplicates pending requests by identifier, so
repeated test sends would otherwise show only one banner.

`ensure_bundled_app` checks that the executable sits at
`<Name>.app/Contents/MacOS/<exe>`. `UNUserNotificationCenter` aborts an
unbundled process instead of returning an error, and `cargo tauri dev` runs
exactly that shape, so the commands return an error in dev rather than
crashing.

On Windows and Linux there is no OS-level prompt to check, so both permission
commands return `granted` and `show_native_notification` delegates to the
plugin.

## Layout

| Path | What it does |
| --- | --- |
| [`mod.rs`](mod.rs) | The three commands and the inline `macos` submodule (`permission_state`, `request_permission`, `show`, bundle check, status mapping). |
| [`macos/native_notifications_tests.rs`](macos/native_notifications_tests.rs) | Tests for the inline `macos` submodule. The directory holds only this file because the submodule itself is defined inside `mod.rs`. |

## Tauri commands

| Command | Returns |
| --- | --- |
| `notification_permission_state` | `granted`, `denied`, `not_determined`, `provisional`, `ephemeral` or `unknown` on macOS; always `granted` elsewhere. |
| `notification_permission_request` | `granted` or `denied` on macOS; always `granted` elsewhere. |
| `show_native_notification(title, body, tag?)` | `Ok(())` once the OS accepted the request; an error string when permission is missing, the bundle check fails, or delivery fails. |

## Boundaries

- Which events become notifications, and when, is decided by the frontend and
  the core. This module only delivers.
- `start_core_process` in `lib.rs` shows its port-fallback notice through the
  plugin directly, not through this module.
- The plugin is still registered in `lib.rs`, and the capability grants its
  `notification:*` permissions.

## Tests

```bash
cargo test --manifest-path crates/openhuman-app/Cargo.toml native_notifications::
```

The tests compile only on macOS.

## Further reading

- [`docs/gitbooks/en/developing/architecture/tauri-shell.md`](../../../../docs/gitbooks/en/developing/architecture/tauri-shell.md): the Tauri shell.
- [`crates/openhuman-app/README.md`](../../README.md): the openhuman-app crate README.
