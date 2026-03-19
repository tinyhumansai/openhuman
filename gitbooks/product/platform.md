---
icon: layer-plus
---

# Platform & Availability

OpenHuman is a native application that runs on six platforms from a single codebase. It is not a web-only tool, browser extension, or Electron wrapper. It is built for performance, security, and a small footprint on every device you use.

***

## Six Platforms, One Experience

OpenHuman compiles to native binaries for each supported platform:

| Platform    | Architectures        | Distribution   |
| ----------- | -------------------- | -------------- |
| **macOS**   | Intel, Apple Silicon | .dmg installer |
| **Windows** | x64, ARM64           | .msi installer |
| **Linux**   | x64, ARM64           | AppImage, .deb |
| **Android** | ARM                  | .apk package   |
| **iOS**     | ARM64                | App Store      |
| **Web**     | Any browser          | Direct access  |

Your account, connected sources, preferences, and settings sync across all platforms. You can start a request on your desktop and review the output on your phone.

***

## Why Native Matters

OpenHuman is built as a native application rather than a web wrapper for three reasons.

**Small footprint.** The app is lightweight. A fraction of the size of typical communication tools. It starts in under a second and uses minimal memory, so it stays out of the way when running alongside other demanding applications.

**Fast startup.** There is no browser engine to initialize. The app launches quickly and is ready to accept requests immediately.

**OS-level security.** On desktop platforms, OpenHuman stores credentials in your operating system's secure keychain (macOS Keychain, Windows Credential Manager, Linux Secret Service). Sensitive data never sits in browser storage or plain text files.

***

## Architecture at a Glance

OpenHuman operates across three layers:

**Application layer.** The native app on your device handles the interface, user input, local state, credential management, and skill execution. This layer is responsible for everything you see and interact with.

**Intelligence layer.** OpenHuman's analysis, coordination, and intelligence systems run as a secure backend service. When a request requires deeper language processing, it is handled here. This layer is operated and maintained by OpenHuman.

**External services.** Connected tools and platforms:Telegram, Notion, Google Sheets, and others are accessed only when you explicitly request it. OpenHuman acts as a bridge between your sources and the intelligence layer, not as a replacement for any of them.

{% hint style="info" %}
The intelligence layer is not part of the client application. It performs analysis, coordination, and trust scoring separately from the frontend.
{% endhint %}

***

## Real-Time Communication

OpenHuman maintains a persistent connection between the application and the intelligence layer. This means responses arrive in real time as they are generated. You see outputs streaming, not loading.

The connection is designed for resilience. If the network drops, OpenHuman reconnects automatically with progressive backoff. There is no manual reconnection process.

***

## Offline Behavior

OpenHuman's local state persists on your device. Your preferences, settings, and connected source configurations remain available even when you are offline.

Full analysis and intelligence features require a network connection, since they depend on the intelligence layer. When connectivity is restored, the app resumes normal operation without requiring you to re-authenticate or reconfigure.
