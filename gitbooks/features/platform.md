---
description: >-
  What OpenHuman ships as (native React + Tauri v2 desktop app with a Rust
  core), supported platforms, and what's in scope today.
icon: layer-plus
---

# Platform & Availability

OpenHuman is a native desktop application, not a browser extension, not an Electron wrapper. Built on **React + Tauri v2** with a **Rust core**, it ships small, starts fast, and stays out of the way.

---

## Supported platforms

| Platform    | Architectures        | Distribution                                   |
| ----------- | -------------------- | ---------------------------------------------- |
| **macOS**   | Intel, Apple Silicon | `.dmg` installer, Homebrew                     |
| **Windows** | x64, ARM64           | `.msi` installer                               |
| **Linux**   | x64                  | AppImage, `.deb`, AUR recipe (`openhuman-bin`) |

### Linux AppImage notes

The Linux AppImage is built for x64 desktops and is the default asset selected
by the curl installer. On newer distributions, especially builds that tighten
unprivileged user namespaces or AppArmor defaults, AppImage startup can fail
before OpenHuman reaches its own crash reporter. Known symptoms include:

- `unshare: write failed /proc/self/uid_map: Operation not permitted`
- `Interpreter not found!`
- `cannot execute binary file`

When that happens, prefer the `.deb` package on Debian/Ubuntu systems. For
Fedora, openSUSE, and other non-Debian distributions, include the distro
version, kernel version, GPU/driver stack, and the exact AppImage filename when
reporting the issue so maintainers can distinguish host restrictions from a
badly packaged AppImage runtime.

---

## Why native matters

OpenHuman is built as a native application rather than a web wrapper for three reasons.

**Small footprint.** A fraction of the size of typical communication tools. Starts in under a second and uses minimal memory.

**Fast startup.** No browser engine to initialize. Ready to accept requests immediately.

**OS-level security.** Credentials live in your platform's secure keychain, macOS Keychain, Windows Credential Manager, Linux Secret Service. Sensitive data never sits in browser storage or plain text files. Memory items live in the engine you select; the compiled `context.md` lives in your workspace folder.

---

## Architecture at a glance

```
┌──────────────────────────────────────────────────┐
│ Tauri shell - windowing, OS integration │
└──────────────────────────────────────────────────┘
 │ JSON-RPC ↕
┌──────────────────────────────────────────────────┐
│ Rust core (`openhuman` sidecar) │
│ • Memory, integrations, source sync │
│ • Model router, TokenJuice, native tools │
│ • Voice (STT in, TTS out, Meet agent) │
└──────────────────────────────────────────────────┘
 │
┌──────────────────────────────────────────────────┐
│ React frontend - screens, navigation │
└──────────────────────────────────────────────────┘
```

The shell is a delivery vehicle (windowing, process lifecycle, IPC). All product logic lives in the Rust core. The React frontend talks to the core over JSON-RPC. See [Architecture](../developing/architecture/) for the full picture.

---

## Remote/headless usage

Linux servers can host the Rust core without a desktop session. The production
shape is a remote `openhuman-core` JSON-RPC service plus a local desktop client
configured with that core URL and bearer token.

A private browser UI is possible for development/preview by serving the Vite
frontend and pointing it at the remote core, but it is not a full replacement
for the desktop shell. Native deep links, tray controls, OS keychain access, CEF
account scanners, and screen/window integrations still require the Tauri app.
See [Cloud Deploy](cloud-deploy.md#remote-ui-choices) for the current remote UI
setup.

---

## Real-time communication

The desktop app maintains a persistent connection to the OpenHuman backend. Responses stream as they are generated; outputs appear progressively, not after a hang. If the network drops, the app reconnects automatically with progressive backoff.

---

## Offline behavior

Your local state persists on your device. Preferences, settings, and connected-source configurations remain available offline. Your workspace files, including the compiled `context.md`, stay readable without a network connection. The [memory engine](memory.md) itself needs connectivity unless you point CortexDB at a local endpoint.

Source sync and live LLM calls require connectivity. When the network returns, the next scheduled sync picks up where it left off.

---

## Auto-update

The desktop shell auto-updates itself via Tauri's updater plugin against a manifest published on GitHub Releases. The OpenHuman core sidecar ships inside the same bundle, so a shell update upgrades both.
