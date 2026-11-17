# capabilities

Tauri v2 capability files. A capability says which windows may use which
permissions. This directory has one, [`default.json`](default.json), and `tauri_build` picks
it up at build time ([`build.rs`](../build.rs) reruns when the directory changes).

## default.json

The `default` capability applies to the `main` window on Linux,
macOS and Windows. It grants:

| Permission | What it allows |
| --- | --- |
| `core:default`, `core:event:default`, `core:window:default` | Tauri's core defaults for windows and events. |
| `core:window:allow-hide`, `allow-show`, `allow-set-focus`, `allow-unminimize`, `allow-start-dragging`, `allow-minimize`, `allow-toggle-maximize`, `allow-set-always-on-top` | Window controls used by the custom titlebar. |
| `deep-link:default` | `openhuman://` deep-link events. |
| `notification:default`, `allow-is-permission-granted`, `allow-request-permission`, `allow-notify` | The notification plugin. On macOS the app's own `native_notifications` commands give the real permission state; see [`../src/native_notifications/`](../src/native_notifications/README.md). |
| `opener:default`, `opener:allow-reveal-item-in-dir` | Opening files and revealing them in the file manager. |
| `opener:allow-open-url` (scoped) | Opening only these URLs from the renderer: `obsidian://open*`, `https://ollama.com/*`, the macOS Accessibility and Screen Recording privacy panes, and `ms-settings:privacy` on Windows. |
| `updater:default` | The updater plugin. |
| `allow-core-process`, `allow-workspace-files`, `allow-artifact-download`, `allow-artifact-save`, `allow-directory-picker`, `allow-app-update`, `allow-loopback-oauth` | The app's own command sets, defined in [`../permissions/`](../permissions/README.md). |

The `$schema` field points at `../gen/schemas/desktop-schema.json`, which
`tauri_build` generates; it is not checked in.

## Gotchas

- Granting a permission here does nothing for a command that no permission
  file lists. App commands are ACL-checked, so a new command needs an entry in
  [`permissions/`](../permissions/) as well as in `generate_handler!`.
- Widen the `opener:allow-open-url` scope by adding an entry, not by switching
  to an unscoped permission. Ordinary `http(s)` links from the main webview
  are handled by `external_navigation.rs`, which opens them in the default
  browser.

## Further reading

- [`docs/gitbooks/en/developing/architecture/tauri-shell.md`](../../../docs/gitbooks/en/developing/architecture/tauri-shell.md): the Tauri shell.
- [`crates/openhuman-app/README.md`](../README.md): the openhuman-app crate README.
