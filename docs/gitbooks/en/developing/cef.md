---
description: >-
  Why the desktop shell no longer uses Chromium (CEF), what replaced it, and
  the rule against injecting JavaScript into child webviews.
icon: chrome
---

# Chromium Embedded Framework (retired)

OpenHuman no longer ships its own Chromium. The desktop shell, `crates/openhuman-app/`, runs on the native webview that Tauri v2 provides through Wry: WKWebView on macOS, WebView2 on Windows and WebKitGTK on Linux. Do not restore CEF or Chrome DevTools Protocol (CDP) scanner assumptions. [`AGENTS.md`](https://github.com/tinyhumansai/openhuman/blob/main/AGENTS.md) states this rule.

This page is background for anyone who finds CEF references in older code or notes.

## What CEF was for

For a while the shell shipped Chromium through a fork of `tauri-runtime`. The stock native webviews do not expose CDP, and CDP was what let the app watch connected web apps such as Slack, WhatsApp, Telegram, Discord and Google Meet. Each one ran in its own child webview with isolated storage, and a scanner for each read its state over CDP rather than through injected JavaScript. The same runtime also powered a Google Meet camera that showed the mascot, and notification interception for embedded apps.

None of that is in the shipped app.

## What replaced it

- Browser automation moved out of the shell. A real Chrome is driven over CDP from the Rust side, through the `tinycomputer` module, as a separate process that the agent launches or attaches to. See [Browser and computer control](../features/native-tools/browser-and-computer.md).
- The scanners that walked third-party apps are gone. The iMessage scanner remains because it reads `~/Library/Messages/chat.db` directly and never needed a browser.

## No JavaScript injection into child webviews

Do not add JavaScript injection to child webviews. New behavior belongs in Rust-side IPC hooks, and any new Tauri plugin should be audited for `js_init_script`.

Host-controlled code that runs inside a third-party origin is an attack-surface risk. A persistent bridge inside another site breaks when that site updates, and one mistake can expose the bridge to attacker-controlled JavaScript.

## See also

- [Tauri shell](architecture/tauri-shell.md): the current desktop host and its IPC surface.
- [`AGENTS.md`](https://github.com/tinyhumansai/openhuman/blob/main/AGENTS.md): the repo-wide rules.
