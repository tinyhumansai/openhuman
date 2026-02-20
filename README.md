<h1 align="center">AlphaHuman Mk1</h1>

<p align="center">
  <strong>Your most productive co-worker</strong><br>
  A user-friendly (GUI-first) AI agent. AlphaHuman uses the
  Neocortex Mk1 model to co-ordinate memories &
  realtime-data, cheaper and faster than other models.
</p>

<p align="center">
  <img src="https://img.shields.io/badge/status-early%20beta-orange" alt="Early Beta" />
  <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux%20%7C%20Android%20%7C%20iOS-blue" alt="Platforms" />
  <a href="https://github.com/alphahumanai/alphahuman/releases/latest"><img src="https://img.shields.io/github/v/release/alphahumanai/alphahuman?label=latest" alt="Latest Release" /></a>
</p>

<p align="center">
  <a href="#what-is-alphahuman">About</a> ·
  <a href="#alphahuman-vs-openclaw">vs OpenClaw</a> ·
  <a href="#download">Download</a> ·
  <a href="#getting-started">Getting Started</a> ·
  <a href="docs/ARCHITECTURE.md">Architecture</a> ·
  <a href="CHANGELOG.md">Changelog</a>
</p>

![The Tet](./docs/the-tet.png)

<p align="center" style="font-style: italic">
  "The Tet. What a brilliant machine" — Morgan Freeman in <a href="https://youtu.be/SveLVpqy_Rc?si=y83aZNokPiUjILN0&t=60">Oblivion</a>
</p>

AlphaHuman is a personal AI assistant that helps you manage high-volume communication without reading everything yourself. It connects to your messaging platforms and productivity tools, understands conversations in context, and produces clear, actionable outputs you can use immediately.

AlphaHuman is **not** a chatbot, browser extension, or cloud-only service. It is a **native application** that runs on your device, connects to your tools, and works only when you ask it to. Think of it as a second brain that sits across your communication and productivity stack.

## AlphaHuman vs OpenClaw

AlphaHuman is designed to be simpler to deploy, cheaper to run, and more intelligent in how it uses models and memory.

|                  | OpenClaw                                                | AlphaHuman                                                                                                  |
| ---------------- | ------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| **Runtime**      | Node.js (TypeScript)                                    | Tauri (Rust + React), native binary                                                                         |
| **Inference**    | Single-tier or manual routing                           | **Custom two-tier**: task-routed (summarize/vibe/memory → cheap; complex/tools → premium)                   |
| **Memory**       | Often external (Pinecone, Lucid, etc.) or markdown-only | **Custom hybrid**: SQLite FTS5 + vector similarity, optional encryption, no external vector DB              |
| **Tunneling**    | Third-party (ngrok, Cloudflare, Tailscale) or none      | **Custom tunneling** — secure app-to-backend path without vendor lock-in                                    |
| **Cost**         | Typically one premium model for everything              | **Lower** — Tier 1 for most ops; Tier 2 only when needed                                                    |
| **Intelligence** | General-purpose agent loop                              | **Smarter** — vibe detection, interest-based escalation, constitution-driven behavior, session-aware memory |
| **Deployment**   | Server/Node process, high memory footprint              | Native desktop/mobile app, Rust socket manager, smaller footprint                                           |

> OpenClaw is a strong open-source agent framework. We chose to build a custom stack so we could own inference routing, memory, and tunneling end-to-end and optimize for cost and clarity.

---

## Download

> **Early Beta** — AlphaHuman is under active development. Expect rough edges.

| Platform    | Variant                     | Download                                                                                                      |
| ----------- | --------------------------- | ------------------------------------------------------------------------------------------------------------- |
| **macOS**   | Apple Silicon (M1/M2/M3/M4) | [`.dmg` (aarch64)](https://github.com/alphahumanai/alphahuman/releases/latest/download/AlphaHuman_aarch64.dmg) |
| **macOS**   | Intel                       | [`.dmg` (x64)](https://github.com/alphahumanai/alphahuman/releases/latest/download/AlphaHuman_x64.dmg)         |
| **Windows** | x64                         | [`.msi`](https://github.com/alphahumanai/alphahuman/releases/latest/download/AlphaHuman_x64_en-US.msi)         |
| **Linux**   | Debian / Ubuntu             | [`.deb` (amd64)](https://github.com/alphahumanai/alphahuman/releases/latest/download/AlphaHuman_amd64.deb)     |
| **Linux**   | Fedora / RHEL               | [`.rpm` (x86_64)](https://github.com/alphahumanai/alphahuman/releases/latest/download/AlphaHuman_x86_64.rpm)   |
| **Linux**   | Universal                   | [`.AppImage`](https://github.com/alphahumanai/alphahuman/releases/latest/download/AlphaHuman_amd64.AppImage)   |
| **Android** | —                           | Coming soon                                                                                                   |
| **iOS**     | —                           | Coming soon                                                                                                   |

Browse all releases: [github.com/alphahumanai/alphahuman/releases](https://github.com/alphahumanai/alphahuman/releases)

## Getting Started

1. **Download** the installer for your platform from the [releases page](https://github.com/alphahumanai/alphahuman/releases/latest)
2. **Install** the app (drag to Applications on macOS, or use your package manager on Linux)
3. **Connect a source** — follow the in-app onboarding to link Telegram, Notion, Gmail, or other services
4. **Run your first request** — ask the AI to summarize what you missed, extract action items, or surface key decisions

---

## Links

- [Architecture Overview](docs/ARCHITECTURE.md) — How AlphaHuman is built
- [Changelog](CHANGELOG.md) — Release history
- [Website](https://alphahuman.xyz) — Learn more

---

<p align="center">
  Made with love in India 🇮🇳
</p>

<p align="center">
  <sub>AlphaHuman is in early beta. Features may change, break, or disappear. Use at your own risk.</sub>
</p>
