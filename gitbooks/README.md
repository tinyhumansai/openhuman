---
description: >-
  Personal AI assistant for your desktop. Connects to 118+ services, builds a
  local-first memory of your life, self-reflects, and can interact with you
  over audio and video.
icon: diamond
---

# Welcome to OpenHuman

<figure><img src=".gitbook/assets/demo.png" alt=""><figcaption></figcaption></figure>

OpenHuman is an open-source AI assistant designed to be the **memory** and **doer** for everything you do across your tools. Built on Rust + Tauri and licensed under GNU GPL3, it closes the gap between what AI models can do and what they actually know about _you_.

Every model in the world, all 200+ of them, shares the same fundamental limitation: they are stateless. You type a prompt, get a response, and the context evaporates. Even the ones with "memory" store a few bullet points. A few bullet points is a sticky note, not intelligence.

OpenHuman solves this with a stack that's calmly, deliberately different:

* **A local-first** [**Memory Tree**](features/obsidian-wiki/memory-tree.md)**.** Every source you connect. Gmail, Slack, GitHub, Notion, your own notes, flows through a deterministic pipeline: canonical Markdown, ≤3k-token chunks, scored, folded into per-source / per-topic / per-day summary trees. Stored in SQLite on your machine. No vector-soup black box.
* **An** [**Obsidian-style wiki**](features/obsidian-wiki/) **on top of it.** The same chunks the agent reasons over land as `.md` files in a vault you can open in [Obsidian](https://obsidian.md), browse, edit, and link by hand. Inspired by [Karpathy's obsidian-wiki workflow](https://x.com/karpathy/status/2039805659525644595). You can't trust a memory you can't read.
* [**118+ third-party integrations**](features/integrations/README.md)**.** One-click OAuth into Gmail, GitHub, Slack, Notion, Stripe, Calendar, Drive, Linear, Jira and more - no API keys to wire by hand, no plugin marketplace to navigate.
* [**Auto-fetch**](features/obsidian-wiki/auto-fetch.md)**.** Every twenty minutes, OpenHuman pulls fresh data from every active connection and folds it into the Memory Tree without you asking, so the agent already has tomorrow's context this morning.
* **An agent built for big data.** [Smart token compression (TokenJuice)](features/token-compression.md) compacts verbose tool output before it ever enters the model's context, so sweeping through your last six months of email costs single-digit dollars. [Automatic model routing](features/model-routing/) sends each task to the right model - `hint:reasoning` to a frontier model, `hint:fast` to a cheap one, vision to vision - all under one subscription. Optional [local AI via Ollama or LM Studio](features/model-routing/local-ai.md) keeps supported workloads on-device.
* [**Batteries included**](features/native-tools/)**.** A complete agent toolbelt is wired in by default: [web search](features/native-tools/web-search.md), a [web-fetch scraper](features/native-tools/web-scraper.md), a full [coder toolset](features/native-tools/coder.md) (filesystem, git, lint, test, grep), [browser & computer control](features/native-tools/browser-and-computer.md), [cron & scheduling](features/native-tools/cron.md), [memory tools](features/native-tools/memory-tools.md), [agent coordination](features/native-tools/agent-coordination.md) for spawning sub-agents, and [native voice](features/native-tools/voice.md) - STT in, TTS out, mascot lip-sync, and a live Google Meet agent that joins meetings, transcribes them into your Memory Tree, and can speak back into the call. No "install a plugin to read files" friction.
* **Simple, UI-first.** A clean desktop experience and short onboarding paths take you from install to a working agent in a few clicks - no config-first setup, no terminal required. The agent has [a face](features/mascot.md): a desktop mascot that speaks, reacts to its surroundings, joins your Google Meets as a real participant, remembers you across weeks, and keeps thinking in the background even when you've stopped typing.

Together, these turn OpenHuman into something fundamentally different from a chatbot. It is an AI agent that consumes large amounts of personal data at low cost, maintains a persistent and evolving understanding of your world, and takes proactive actions on your behalf.

{% hint style="warning" %}
OpenHuman is not AGI. But it is a meaningful architectural step closer, with better memory, better orchestration, and better tooling.
{% endhint %}
