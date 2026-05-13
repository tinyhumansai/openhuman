---
description: >-
  What OpenHuman ships as (native React + Tauri v2 desktop app with a Rust
  core), supported platforms, and what's in scope today.
icon: layer-plus
---

# Platform & Availability

OpenHuman is a native desktop application, not a browser extension, not an Electron wrapper. Built on **React + Tauri v2** with a **Rust core**, it ships small, starts fast, and stays out of the way.

***

## Supported platforms

| Platform    | Architectures        | Distribution               |
| ----------- | -------------------- | -------------------------- |
| **macOS**   | Intel, Apple Silicon | `.dmg` installer, Homebrew |
| **Windows** | x64, ARM64           | `.msi` installer           |
| **Linux**   | x64                  | AppImage, `.deb`           |

***

## Why native matters

OpenHuman is built as a native application rather than a web wrapper for three reasons.

**Small footprint.** A fraction of the size of typical communication tools. Starts in under a second and uses minimal memory.

**Fast startup.** No browser engine to initialize. Ready to accept requests immediately.

**OS-level security.** Credentials live in your platform's secure keychain, macOS Keychain, Windows Credential Manager, Linux Secret Service. Sensitive data never sits in browser storage or plain text files. The local Memory Tree's SQLite database lives in your workspace folder, owned by you.

***

## Architecture at a glance

```
┌──────────────────────────────────────────────────┐
│ Tauri shell - windowing, OS integration │
└──────────────────────────────────────────────────┘
 │ JSON-RPC ↕
┌──────────────────────────────────────────────────┐
│ Rust core (`openhuman` sidecar) │
│ • Memory Tree, integrations, auto-fetch │
│ • Model router, TokenJuice, native tools │
│ • Voice (STT in, TTS out, Meet agent) │
└──────────────────────────────────────────────────┘
 │
┌──────────────────────────────────────────────────┐
│ React frontend - screens, navigation │
└──────────────────────────────────────────────────┘
```

The shell is a delivery vehicle (windowing, process lifecycle, IPC). All product logic lives in the Rust core. The React frontend talks to the core over JSON-RPC. See [Architecture](../developing/architecture/) for the full picture.

***

## Real-time communication

The desktop app maintains a persistent connection to the OpenHuman backend. Responses stream as they are generated; outputs appear progressively, not after a hang. If the network drops, the app reconnects automatically with progressive backoff.

***

## Offline behavior

Your local state persists on your device. Preferences, settings, and connected-source configurations remain available offline. The local Memory Tree is fully accessible, you can browse the [Obsidian vault](obsidian-wiki/) and read your existing notes without any network connection.

Auto-fetch and live LLM calls require connectivity. When the network returns, the next 20-minute tick picks up where it left off.

***

## Auto-update

The desktop shell auto-updates itself via Tauri's updater plugin against a manifest published on GitHub Releases. The OpenHuman core sidecar ships inside the same bundle, so a shell update upgrades both.
