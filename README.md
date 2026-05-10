<h1 align="center">OpenHuman</h1>

<p align="center">
 <img src="./gitbooks/.gitbook/assets/demo.png" alt="The Tet" />
</p>

<p align="center" style="display: inline-block">
 <a href="https://trendshift.io/repositories/23680" target="_blank" style="display: inline-block">
  <img src="https://trendshift.io/api/badge/repositories/23680" alt="tinyhumansai%2Fopenhuman | Trendshift" style="width: 250px; height: 55px;" width="250" height="55"/>
 </a>
</p>

<p align="center">
 <strong>OpenHuman is your Personal AI super intelligence. Private, Simple and extremely powerful.</strong>
</p>


<p align="center">
 <a href="https://discord.tinyhumans.ai/">Discord</a> •
 <a href="https://www.reddit.com/r/tinyhumansai/">Reddit</a> •
 <a href="https://x.com/intent/follow?screen_name=tinyhumansai">X/Twitter</a> •
 <a href="https://tinyhumans.gitbook.io/openhuman/">Docs</a> •
 <a href="https://x.com/intent/follow?screen_name=senamakel">Follow @senamakel (Creator)</a>
</p>

<p align="center">
 <img src="https://img.shields.io/badge/status-early%20beta-orange" alt="Early Beta" />
 <a href="https://github.com/tinyhumansai/openhuman/releases/latest"><img src="https://img.shields.io/github/v/release/tinyhumansai/openhuman?label=latest" alt="Latest Release" /></a>
</p>

> **Early Beta**: Under active development. Expect rough edges.

To install or get started, either download from the website over at [tinyhumans.ai/openhuman](https://tinyhumans.ai/openhuman) or run

```
# Download DMG, EXEs over at https://tinyhumans.ai/openhuman or run in from your terminal

# For MacOS/Linux
curl -fsSL https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.sh | bash

# For Windows
irm https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.ps1 | iex
```

# What is OpenHuman?

OpenHuman is an open-source agentic assistant designed to integrate with you in your daily life. Each bullet links to the deeper writeup in the [docs](https://tinyhumans.gitbook.io/openhuman/).

- **Simple, UI-first & Human** A clean desktop experience and short onboarding paths take you from install to a working agent in a few clicks — no config-first setup, no terminal required. The agent has [a face](https://tinyhumans.gitbook.io/openhuman/features/mascot): a desktop mascot that speaks, reacts to its surroundings, [joins your Google Meets](https://tinyhumans.gitbook.io/openhuman/features/mascot/meeting-agents) as a real participant, remembers you across weeks, and keeps thinking in the background even when you've stopped typing.

- **[118+ third-party integrations](https://tinyhumans.gitbook.io/openhuman/features/integrations) with [auto-fetch](https://tinyhumans.gitbook.io/openhuman/features/obsidian-wiki/auto-fetch)**: plug into Gmail, Notion, GitHub, Slack, Stripe, Calendar, Drive, Linear, Jira and the rest of your stack with **one-click OAuth**. Every connection is exposed to the agent as a typed tool, and every twenty minutes the core walks each active connection and pulls fresh data into the [memory tree](https://tinyhumans.gitbook.io/openhuman/features/integrations/auto-fetch). No prompts, no polling loops you have to write, so the agent already has tomorrow's context this morning.

- **[Memory Tree](https://tinyhumans.gitbook.io/openhuman/features/memory-tree) + [Obsidian Wiki](https://tinyhumans.gitbook.io/openhuman/features/obsidian-wiki)**: a local-first knowledge base built from your data and your activity. Everything you connect is canonicalized into ≤3k-token Markdown chunks, scored, and folded into hierarchical summary trees stored in **SQLite on your machine**. The same chunks land as `.md` files in an Obsidian-compatible vault you can open, browse and edit, inspired by Karpathy's [obsidian-wiki workflow](https://x.com/karpathy/status/2039805659525644595).

- **Batteries included**: web search, a web-fetch [scraper](https://tinyhumans.gitbook.io/openhuman/features/native-tools), a full coder toolset (filesystem, git, lint, test, grep), and [native voice](https://tinyhumans.gitbook.io/openhuman/features/voice) (STT in, ElevenLabs TTS out, mascot lip-sync, live Google Meet agent) are wired in by default. [Model routing](https://tinyhumans.gitbook.io/openhuman/features/model-routing) sends each task to the right LLM (reasoning, fast, or vision) under one subscription. No "install a plugin to read files" friction. [Optional local AI via Ollama](https://tinyhumans.gitbook.io/openhuman/features/model-routing/local-ai) for on-device workloads.

- **[Smart token compression (TokenJuice)](https://tinyhumans.gitbook.io/openhuman/features/token-compression)**: every tool call, scrape result, email body, and search payload is run through a token compression layer before it touches any LLM Model. HTML is converted to Markdown, long URLs are shortened, non-Asccii characters are removed etc... You get the same information but at a fraction of the tokens. Reducing costs &amp; increasing latency by upto 80%.

- **[Messaging channels](https://tinyhumans.gitbook.io/openhuman/features/integrations#messaging-channels)** and **[privacy & security](https://tinyhumans.gitbook.io/openhuman/features/privacy-and-security)**: inbound/outbound across the channels you already use, with workflow data that stays on device, encrypted locally, treated as yours.

For contributors: Read the [Architecture](https://tinyhumans.gitbook.io/openhuman/developing/architecture) · [Getting Set Up](https://tinyhumans.gitbook.io/openhuman/developing/getting-set-up) · [Cloud Deploy](https://tinyhumans.gitbook.io/openhuman/developing/cloud-deploy) · [`CONTRIBUTING.md`](./CONTRIBUTING.md).

## Context in minutes, not weeks

OpenHuman is the first agent harness that gets to know you in minutes. Inspired by [Karpathy's LLM Knowledgebase](https://x.com/karpathy/status/2039805659525644595). Most agents start cold. Hermes learns by watching you work; OpenClaw waits for plugins to ferry context in. Either way, you spend days or weeks before the agent knows enough about your stack to be genuinely useful.

<p align="center">
 <img src="./gitbooks/.gitbook/assets/image.png" />
</p>

> OpenHuman summarizes and compresses all your documents, emails & chats; and creates a memory graph that lets your agent remember everything about you.

OpenHuman skips the wait. Connect your accounts, let [auto-fetch](https://tinyhumans.gitbook.io/openhuman/features/integrations/auto-fetch) pull data locally on a 20-minute loop, and then have [Memory Trees](https://tinyhumans.gitbook.io/openhuman/features/memory-tree) compresses everything into Markdown files stored intelligently in a [Karpathy-style Obsidian wiki](https://tinyhumans.gitbook.io/openhuman/features/obsidian-wiki).

In just one sync pass and the agent has full (compressed) context your inbox, your calendar, your repos, your docs, your messages. No training period. No "give it a few weeks.". It becomes you, controlled by you.

## OpenHuman vs Other Agent Harnesses

High-level comparison (products evolve, so verify against each vendor). OpenHuman is built to **minimize vendor sprawl**, keep **workflow knowledge on-device**, and give the agent a **persistent memory** of your data, not only chat.

|                     | Claude Cowork     | OpenClaw          | Hermes Agent      | OpenHuman                          |
| ------------------- | ----------------- | ----------------- | ----------------- | ---------------------------------- |
| **Open-source**     | 🚫 Proprietary    | ✅ MIT            | ✅ MIT            | ✅ GNU                             |
| **Simple to start** | ✅ Desktop + CLI  | ⚠️ Terminal-first | ⚠️ Terminal-first | ✅ Clean UI, minutes               |
| **Cost**            | ⚠️ Sub + add-ons  | ⚠️ BYO models     | ⚠️ BYO models     | ✅ One sub + TokenJuice            |
| **Memory**          | ✅ Chat-scoped    | ⚠️ Plugin-reliant | ✅ Self-learning  | 🚀 Memory Tree + Obsidian vault    |
| **Integrations**    | ⚠️ Few connectors | ⚠️ BYO            | ⚠️ BYO            | 🚀 118+ via OAuth                  |
| **Auto-fetch**      | 🚫 None           | 🚫 None           | 🚫 None           | ✅ 20-min sync into memory         |
| **API sprawl**      | 🚫 Extra keys     | 🚫 BYOK           | 🚫 Multi-vendor   | ✅ One account                     |
| **Model routing**   | 🚫 Single model   | ⚠️ Manual         | ⚠️ Manual         | ✅ Built-in                        |
| **Native tools**    | ✅ Code-only      | ✅ Code-only      | ✅ Code-only      | ✅ Code + search + scraper + voice |

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

Show some love and end up in the hall of fame. Contributors get free merch and special access to our [Discord](https://discord.tinyhumans.ai/).

<a href="https://github.com/tinyhumansai/openhuman/graphs/contributors">
 <img src="https://contrib.rocks/image?repo=tinyhumansai/openhuman" alt="OpenHuman contributors" />
</a>
