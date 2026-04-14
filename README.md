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

> **Early Beta** — Under active development. Expect rough edges - we're improving daily!

To install or get started, either download from the website over at [tinyhumans.ai/openhuman](https://tinyhumans.ai/openhuman) or run

```
# For MacOS/Linux
curl -fsSL https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.sh | bash

# For Windows
irm https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.ps1 | iex
```

# What is OpenHuman?

OpenHuman is an open-source agentic assistant that is designed to integrate with you in your daily life. Here's what makes OpenHuman special:

- **Simple, UI-first** — A **clean** desktop experience and short onboarding paths so you can go from install to a **working agent in a few clicks**, without a config-first setup. You don't need a terminal to run OpenHuman.

- **One subscription, many providers** — You only need **one** account to get access to many agentic APIs (AI Models, Search, Webhooks/Tunnels and other 3rd party APIs etc..), simplifying the experience to get a powerful agent going.

- **Rich Skills** — Plug into **Gmail**, **Slack**, **Notion**, and the rest of your stack via **rich, feature-backed skills**. Connections are typically **one click** through setup wizards instead of wiring APIs by hand. Workflow data is kept **on device**, **encrypted locally**, and treated as **yours**: encryption and sensitive context stay **on your machine**. **Webhooks** give **instant feedback** into the agent when external systems or skills emit events, so the loop stays tight without constant polling.

- **Local knowledge base** — Built from **your data and your activity**. How you work across tools, sessions, and connected services—so the agent gets **rich, workflow-aware context**, not a one-off chat transcript. Everything is **stored on your machine** and compounding over time without becoming a cloud dossier. **Channels**, **skills** and ongoing **conversations** feed the same loop so day-to-day context does not reset every session.

- **Local AI model** — The **Rust core** exposes **local AI** paths (and the desktop bundle can ship **local/bundled runners** where applicable) for the workloads above—vision snippets, speech helpers, summarization, tooling—so sensitive steps can stay **off the cloud** when you choose.

- **Deep desktop integrations** — OpenHuman is a **native desktop** assistant, not a web-only chat: **memory-aware keyboard autocomplete**, **voice** (**STT** listening and **TTS** replies), **screen intelligence** that understands what is on screen and feeds your local context, plus windowing and OS-level permissions—so the agent meets you **on the machine**, not trapped in a browser tab.

Architecture: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md). Contributor orientation: [`CONTRIBUTING.md`](./CONTRIBUTING.md).

## OpenHuman vs other agents

High-level comparison (products evolve—verify against each vendor). OpenHuman is built to **minimize vendor sprawl**, keep **workflow knowledge on-device**, and ship **deep desktop** features—not only chat.

|                                                                           | Claude Code/Cowork                                     | OpenClaw                                                              | Hermes Agent                                | OpenHuman                                                                                                 |
| ------------------------------------------------------------------------- | ------------------------------------------------------ | --------------------------------------------------------------------- | ------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| **Open-source**: Is the codebase open to review?                          | 🚫 Proprietary client                                  | ✅ MIT License                                                        | ✅ MIT License                              | ✅ GNU License                                                                                            |
| **Simple**: Is it simple to get started?                                  | ✅ Simple Desktop App + CLI                            | ⚠️ Terminal first and often complex                                   | ⚠️ Terminal first and often complex         | ✅ Simple, Clean UI/UX. Get started within minutes                                                        |
| **Cost**: How expensive is to run?                                        | ⚠️ Subscription + **add-on** tool/API costs            | ⚠️ Tied to **models & hosting** you choose                            | ⚠️ Tied to **models & hosting** you choose  | ✅ **Cost optimized** with the option to run many things locally for free                                 |
| **Memory & Knowledge Base (KB)**: Does the agent know you and your world? | ✅ Built-in **memory**; mostly **chat/session** scoped | ⚠️ Has a local memory but often needs **plugins** for richer behavior | ✅ **Self-learning** / task loops (typical) | 🚀 **Local KB + Self-learning** from **your** activity & data (GMail, Notion etc... via skills) & prompts |
| **API spagetti**: How complex is it to hook mulitple features together?   | 🚫 Claude bill + often **extra keys** for MCP/tools    | 🚫 **BYOK** / **multi-vendor** common                                 | 🚫 **Multiple providers** common            | ✅ **One account** get access to many **bundled** platform APIs                                           |
| **Extensibility**: Can you add rich features into it?                     | ✅ **MCP** (different model than sandboxed skills)     | ✅ Plugin Architecture (SKILL.md)                                     | ✅ Plugin Architecture (SKILL.md)           | 🚀 **Rich Skills** with ability to have realtime updates, local DB & more                                 |
| **Desktop integrations**: Can it integrate into your desktop completely?  | ⚠️ Desktop app & access to folders                     | ⚠️ Often **lighter** native surface                                   | ⚠️ Often **lighter** native surface         | ✅ **STT**, **TTS**, **screen intelligence**, **memory-aware autocomplete** and a whole lot more          |

<!-- # Star us on GitHub

_Building toward AGI and artificial consciousness? Star the repo and help others find the path._

<p align="center">
  <a href="https://www.star-history.com/#tinyhumansai/openhuman&type=date&legend=top-left">
    <picture>
     <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=tinyhumansai/openhuman&type=date&theme=dark&legend=top-left" />
     <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/svg?repos=tinyhumansai/openhuman&type=date&legend=top-left" />
     <img alt="Star History Chart" src="https://api.star-history.com/svg?repos=tinyhumansai/openhuman&type=date&legend=top-left" />
    </picture>
  </a>
</p> -->

# Contributors Hall of Fame

Show some love and end up in the hall of fame

<a href="https://github.com/tinyhumansai/openhuman/graphs/contributors">
  <img src="https://contrib.rocks/image?repo=tinyhumansai/openhuman" alt="OpenHuman contributors" />
</a>
## Contributors
- olatundefaruk88-svg
