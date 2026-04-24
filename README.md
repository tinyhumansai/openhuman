<h1 align="center">OpenHuman</h1>

<p align="center">
  <strong>The age of super intelligence is here. OpenHuman is your Personal AI super intelligence. Private, Simple and extremely powerful.</strong>
</p>

<p align="center">
  <a href="https://discord.tinyhumans.ai/">Discord</a> •
  <a href="https://www.reddit.com/r/tinyhumansai/">Reddit</a> •
  <a href="https://x.com/intent/follow?screen_name=tinyhumansai">X/Twitter</a> •
  <a href="https://tinyhumans.gitbook.io/openhuman/">Docs</a>
</p>
<p align="center">
  <a href="https://x.com/intent/follow?screen_name=senamakel">Follow @senamakel (Creator)</a>
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

Architecture: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md). Contributor orientation: [`CONTRIBUTING.md`](./CONTRIBUTING.md). Running from source: [`docs/install.md`](docs/install.md#running-from-source).

## Highlights

- **[Neocortex](https://tinyhumans.gitbook.io/openhuman/technology/neocortex)** — local-first knowledge base that learns from your data and activity, compounding context across tools and sessions.
- **[The Subconscious](https://tinyhumans.gitbook.io/openhuman/technology/the-subconscious)** — background self-learning loops that turn everyday usage into workflow-aware intelligence.
- **[Screen Intelligence](https://tinyhumans.gitbook.io/openhuman/features/screen-intelligence)** — the agent sees what's on your screen and feeds it into your local context.
- **[Inline Autocomplete](https://tinyhumans.gitbook.io/openhuman/features/inline-autocomplete)** — memory-aware keyboard autocomplete anywhere on your desktop.
- **[Voice (STT + TTS)](https://tinyhumans.gitbook.io/openhuman/features/voice-speech-to-text)** — speak to OpenHuman and hear it reply, natively on the desktop.
- **[Skills & Integrations](https://tinyhumans.gitbook.io/openhuman/product/skills-and-integrations)** — one-click skills for Gmail, Slack, Notion and the rest of your stack, with local encryption and webhooks for instant feedback.
- **[Messaging Channels](https://tinyhumans.gitbook.io/openhuman/product/messaging-channels)** — inbound/outbound across the channels you already use, routed through your agent.
- **[Teams & Organizations](https://tinyhumans.gitbook.io/openhuman/product/teams)** — shared workspaces for collaborating with an agent across a team.
- **[Rewards & Achievements](https://tinyhumans.gitbook.io/openhuman/product/rewards-and-achievements)** — gamified progression as your agent grows with you.
- **[Privacy & Security](https://tinyhumans.gitbook.io/openhuman/product/privacy-and-security)** — workflow data stays on device, encrypted locally, and treated as yours.

## OpenHuman vs other agents

High-level comparison (products evolve—verify against each vendor). OpenHuman is built to **minimize vendor sprawl**, keep **workflow knowledge on-device**, and ship **deep desktop** features—not only chat.

|                         | Claude Code/Cowork | OpenClaw          | Hermes Agent      | OpenHuman              |
| ----------------------- | ------------------ | ----------------- | ----------------- | ---------------------- |
| **Open-source**         | 🚫 Proprietary     | ✅ MIT            | ✅ MIT            | ✅ GNU                 |
| **Simple to start**     | ✅ Desktop + CLI   | ⚠️ Terminal-first | ⚠️ Terminal-first | ✅ Clean UI, minutes   |
| **Cost**                | ⚠️ Sub + add-ons   | ⚠️ BYO models     | ⚠️ BYO models     | ✅ Local-friendly      |
| **Memory & KB**         | ✅ Chat-scoped     | ⚠️ Plugin-reliant | ✅ Self-learning  | 🚀 Local KB + learning |
| **API sprawl**          | 🚫 Extra keys      | 🚫 BYOK           | 🚫 Multi-vendor   | ✅ One account         |
| **Extensibility**       | ✅ MCP             | ✅ SKILL.md       | ✅ SKILL.md       | 🚀 Rich Skills         |
| **Desktop integration** | ⚠️ Basic           | ⚠️ Light          | ⚠️ Light          | ✅ STT/TTS/screen/more |

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
