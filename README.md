<h1 align="center">OpenHuman</h1>

<p align="center">
  <strong>The age of super intelligence is here. OpenHuman is your Personal AI super intelligence. Private, Simple and extremely powerful.</strong>
</p>

<p align="center">
  <a href="https://discord.com/invite/k23Kn8nK">Discord</a> •
  <a href="https://www.reddit.com/r/tinyhumansai/">Reddit</a> •
  <a href="https://x.com/tinyhumansai">X/Twitter</a> •
  <a href="https://tinyhumans.gitbook.io/openhuman/">Docs</a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/status-early%20beta-orange" alt="Early Beta" />
  <img src="https://img.shields.io/badge/platform-desktop-macOS%20%7C%20Windows%20%7C%20Linux-blue" alt="Platforms: desktop only" />
  <a href="https://github.com/tinyhumansai/openhuman/releases/latest"><img src="https://img.shields.io/github/v/release/tinyhumansai/openhuman?label=latest" alt="Latest Release" /></a>
</p>

<p align="center">
  <img src="./docs/the-tet.png" alt="The Tet" />
</p>

<p align="center" style="font-style: italic">
  "The Tet. What a brilliant machine" — Morgan Freeman as he reminisces about <a href="https://youtu.be/SveLVpqy_Rc?si=y83aZNokPiUjILN0&t=60">alien superintelligence</a> in the movie <em>Oblivion</em>
</p>

> **Early Beta** — Under active development. Expect rough edges.

OpenHuman is an open-source agentic assistant that is designed to integrate with you in your daily life. Here's what makes OpenHuman special:

- **One subscription, many providers** — One assistant wired to **skills** and backend models so you are not juggling a separate subscription stack for every integration surface.

- **Incredible memory** — **Rust-side memory** (store / recall / namespaces) plus optional **TinyHumans [Neocortex](https://github.com/tinyhumansai/neocortex)**-backed context when configured, so the agent can retain and retrieve more than a single chat window. **Channels** and ongoing **conversations** feed the same loop so day-to-day context does not reset every session.

- **Screen intelligence** — Regular **screen capture** (on a cadence or when triggered) feeds an on-device pipeline that **understands what is on screen**, distills it into **memory** (facts, UI state, workflows), and can propose **actions** the agent executes for you. OS permissions and capture APIs vary by platform; the goal is **your machine first**, not shipping raw frames to the cloud by default.

- **Voice & meetings** — A **Local-model** speech stack (listen / **TTS**) let the assistant **talk back** and **capture or work with meeting audio** with a privacy-first default when you route inference locally. Transcripts and summaries land in the same **memory + agent** loop so OpenHuman can **follow up**: tasks, drafts, calendar nudges, or skill-backed workflows—without treating a meeting as a one-off chat.

- **Memory-aware autocomplete** — **Keyboard autocomplete** is built for **right-context** suggestions: it consults **memory namespaces** and recent context so completions stay aligned with **you**, your workspace, and prior sessions—not a blank model every keystroke.

- **Runs a local AI model** — The **Rust core** exposes **local AI** paths (and the desktop bundle can ship **local/bundled runners** where applicable) for the workloads above—vision snippets, speech helpers, summarization, tooling—so sensitive steps can stay **off the cloud** when you choose.

- **Simple or advanced** — **Skill setup wizards** and defaults for common tools, with room to go deeper via **settings, credentials, and core RPC** when you need control and privacy.

Architecture: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md). Contributor orientation: [`CONTRIBUTING.md`](./CONTRIBUTING.md).

# Download

> **Early Beta** — Under active development. Expect rough edges.

You can download the latest desktop build from the website at [tinyhuman.ai/openhuman](https://tinyhuman.ai/openhuman). You can also grab it from the [latest GitHub release](https://github.com/tinyhumansai/openhuman/releases/latest), which includes all current artifacts (`.dmg`, `.deb`, `.AppImage`, `.app.tar.gz`, and more).

If you need an older version, browse [all releases](https://github.com/tinyhumansai/openhuman/releases).

If you want to build from source, see [`docs/BUILDING.md`](docs/BUILDING.md).

Install with one command:

```
curl -fsSL https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.sh | bash
```

On Windows, use PowerShell:
`irm https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.ps1 | iex`

What setup does:

- Resolves the latest stable release for your OS/arch
- Verifies release digest when available
- Installs locally without requiring system-wide admin rights by default
- macOS: installs `OpenHuman.app` in `~/Applications`
- Linux: installs `openhuman` AppImage in `~/.local/bin/openhuman` and creates a desktop entry
- Windows: installs from latest release MSI/EXE in per-user mode where supported

# Under the hood (Architecture)

OpenHuman is a **desktop monorepo**: **Rust** owns **business logic and execution**; the **UI** owns **interaction, layout, and OS integration**.

**Rust (`openhuman` / `openhuman_core`).** The repo root **`src/`** crate is the brain: **JSON-RPC over HTTP** (`core_server`), domain modules (auth, config, memory, skills, channels, screen intelligence, local AI, cron, …), and a **QuickJS** runtime for **sandboxed JavaScript skills**. The **`openhuman`** binary is built and **staged next to the Tauri app** so the desktop shell can spawn it as a **sidecar**. Heavy work—SQLite, sockets, crypto, skill lifecycle—runs there under **Tokio**, not in the WebView.

**UI (`app/`).** **Vite + React** (TypeScript) implements screens, onboarding, settings, and realtime UX. **Redux Toolkit** holds client state; **Socket.io** and the **MCP-style** client stack stay in sync with the core’s realtime surface. **Tauri v2** (`app/src-tauri/`) is a thin **Rust host**: windowing, filesystem hooks where needed, and **`core_rpc_relay`**—forwarding JSON-RPC from the WebView to the **`openhuman`** process so the UI never re-implements domain rules.

**Controllers and the RPC surface.** Features are exposed as **registered controllers**: each domain declares **schemas** (namespace, function name, parameter shapes) and a **handler**. At runtime, calls are **validated**, dispatched by **method name** (e.g. `openhuman.auth_get_state`, `openhuman.local_ai_agent_chat`), and return structured outcomes. **CLI** and **HTTP** share the same controller catalog, so automation, tests, and the app all hit one contract.

**What ties it together:** one **registry** of controllers, one **sidecar** process for execution, **Tauri IPC** for shell-only capabilities, and **HTTP JSON-RPC** for everything else—plus **skills** and **dual-socket** behavior documented in the architecture guide.

**Read more:** [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) · Frontend tree: [`docs/src/README.md`](docs/src/README.md) · Tauri commands: [`docs/src-tauri/README.md`](docs/src-tauri/README.md)

# Star us on GitHub

_Building toward AGI and artificial consciousness? Star the repo and help others find the path._

<p align="center">
  <a href="https://www.star-history.com/#tinyhumansai/openhuman&type=date&legend=top-left">
    <picture>
     <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=tinyhumansai/openhuman&type=date&theme=dark&legend=top-left" />
     <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/svg?repos=tinyhumansai/openhuman&type=date&legend=top-left" />
     <img alt="Star History Chart" src="https://api.star-history.com/svg?repos=tinyhumansai/openhuman&type=date&legend=top-left" />
    </picture>
  </a>
</p>

# Contributors Hall of Fame

Show some love and end up in the hall of fame

<a href="https://github.com/tinyhumansai/openhuman/graphs/contributors">
  <img src="https://contrib.rocks/image?repo=tinyhumansai/openhuman" alt="OpenHuman contributors" />
</a>
