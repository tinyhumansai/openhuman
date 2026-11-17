---
description: >-
  What OpenHuman ships as (a native desktop app with a Rust core), which
  platforms it supports, and how it behaves offline and when updating.
icon: layer-plus
---

# Platform and availability

OpenHuman is a native desktop app. It is not a browser extension or an Electron wrapper. It is built on React and Tauri v2 with a Rust core, so it ships small, starts fast and stays out of the way.

## Supported platforms

| Platform | Architectures | Distribution |
| --- | --- | --- |
| macOS | Intel, Apple Silicon | `.dmg` installer, Homebrew cask (can lag a release) |
| Windows | x64 | `.msi` or `.exe` installer |
| Linux | x64, ARM64 | `.deb`, AppImage |

Download from [tinyhumans.ai/openhuman](https://tinyhumans.ai/openhuman) or the [latest release](https://github.com/tinyhumansai/openhuman/releases/latest), or use the install scripts, which check the release's SHA-256. See [INSTALL.md](https://github.com/tinyhumansai/openhuman/blob/main/INSTALL.md).

### Linux AppImage notes

On Debian and Ubuntu the install script picks the `.deb`. Elsewhere it picks the AppImage. On newer distributions that tighten unprivileged user namespaces or AppArmor defaults, the AppImage can fail before OpenHuman reaches its own crash reporter. Known symptoms are:

- `unshare: write failed /proc/self/uid_map: Operation not permitted`
- `Interpreter not found!`
- `cannot execute binary file`

If that happens on Debian or Ubuntu, use the `.deb`. On Fedora, openSUSE and other distributions, include the distro version, kernel version, GPU and driver stack, and the exact AppImage filename when you report the problem. That helps maintainers tell host restrictions from a bad AppImage runtime.

The AppImage uses your system's graphics drivers. If your system has `webkit2gtk-4.1` installed and it is at least as new as the bundled copy, the AppImage also uses the system WebKitGTK, so the WebKit build matches your drivers. It prints `using host WebKitGTK from ...` when it does this. To force the bundled copy, launch it with `OPENHUMAN_APPIMAGE_WEBKIT=bundled`.

## Why native matters

Native beats a web wrapper for three reasons.

Small footprint. It is a fraction of the size of typical communication tools and uses little memory.

Fast startup. There is no browser engine to initialize, so it accepts requests right away.

OS-level security. Credentials live in your platform's secure keychain: macOS Keychain, Windows Credential Manager or Linux Secret Service. Sensitive data never sits in browser storage or plain text files. Memory items live in the engine you select.

## Architecture at a glance

```text
┌────────────────────────────────────────────────────────┐
│ Tauri shell (Wry)  ·  windowing, OS integration, IPC   │
│  ┌──────────────────────────────────────────────────┐  │
│  │ Rust core, in-process (no sidecar)               │  │
│  │  • Memory, integrations, source sync             │  │
│  │  • Model router, TokenJuice, native tools        │  │
│  │  • Voice (STT in, TTS out, live voice agent)     │  │
│  └──────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────┘
                      ▲ JSON-RPC over loopback HTTP
                      │
┌────────────────────────────────────────────────────────┐
│ React frontend  ·  screens, navigation                 │
└────────────────────────────────────────────────────────┘
```

The shell handles windowing, process lifecycle and IPC. All product logic lives in the Rust core, which runs as a tokio task inside the shell process, not as a separate binary. The React frontend talks to it over JSON-RPC on loopback. See [Architecture](../developing/architecture/README.md) for more.

## Remote and headless use

A Linux server can host the Rust core without a desktop session. In production, you run a remote `openhuman-core` JSON-RPC service and point a local desktop client at its URL with a bearer token.

For development and preview, you can serve the Vite frontend as a private browser UI that points at the remote core. It does not replace the desktop shell. Native deep links, tray controls, OS keychain access, the native iMessage scanner, and screen and window integrations still need the Tauri app. See [Cloud deploy](cloud-deploy.md#remote-ui-choices) for the remote UI setup.

## Real-time communication

The desktop app keeps a persistent connection to the OpenHuman backend. Responses stream as they are generated, so output appears progressively. If the network drops, the app reconnects automatically with increasing backoff.

## Offline behavior

Your local state stays on your device. Preferences, settings and connected-source configurations remain available offline, and your workspace files stay readable. The [memory engine](memory.md) needs a connection unless you point CortexDB at a local endpoint.

Source sync and live LLM calls need a connection. When the network returns, the next scheduled sync picks up where it left off.

## Auto-update

The desktop shell updates itself through Tauri's updater plugin, using a manifest published on GitHub Releases. The core is linked into the shell, so one update upgrades both. Details, including the retry policy and why a signature failure is never retried, are in [Auto-update](../overview/auto-update.md).
