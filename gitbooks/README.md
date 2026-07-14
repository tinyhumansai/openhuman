---
description: >-
  Personal AI super intelligence for your desktop: a brain that builds a
  local-first memory of your life, a fantastic orchestrator of agent fleets and
  workflows, and a deep researcher across 118+ connected services.
icon: diamond
---

# Welcome to OpenHuman

<figure><img src=".gitbook/assets/demo.png" alt=""><figcaption></figcaption></figure>

OpenHuman is an open-source AI assistant built to be three things most assistants aren't: **a brain** (a persistent, local, readable memory of your world); **a fantastic orchestrator** (durable agent graphs, visual workflows, sub-agent fleets, and [end-to-end encrypted agent-to-agent sessions](features/orchestration.md)); and **a deep researcher** (it sweeps your data and the web before you finish asking). Built on Rust + Tauri, licensed under GNU GPL3.

Every model in the world, all 200+ of them, shares the same fundamental limitation: they are stateless. You type a prompt, get a response, and the context evaporates. Even the ones with "memory" store a few bullet points. A few bullet points is a sticky note, not intelligence.

OpenHuman solves this with a stack that's calmly, deliberately different:

* **A local-first** [**Memory Tree**](features/obsidian-wiki/memory-tree.md)**.** Every source you connect. Gmail, Slack, GitHub, Notion, your own notes, flows through a deterministic pipeline: canonical Markdown, ≤3k-token chunks, scored, folded into per-source / per-topic / per-day summary trees. Stored in SQLite on your machine. No vector-soup black box.
* **An** [**Obsidian-style wiki**](features/obsidian-wiki/) **on top of it.** The same chunks the agent reasons over land as `.md` files in a vault you can open in [Obsidian](https://obsidian.md), browse, edit, and link by hand. Inspired by [Karpathy's obsidian-wiki workflow](https://x.com/karpathy/status/2039805659525644595). You can't trust a memory you can't read.
* [**118+ third-party integrations**](features/integrations/README.md)**.** One-click OAuth into Gmail, GitHub, Slack, Notion, Stripe, Calendar, Drive, Linear, Jira and more - no API keys to wire by hand, no plugin marketplace to navigate.
* [**Auto-fetch**](features/obsidian-wiki/auto-fetch.md)**.** Every twenty minutes, OpenHuman pulls fresh data from every active connection and folds it into the Memory Tree without you asking, so the agent already has tomorrow's context this morning.
* **An agent built for big data.** [Smart token compression (TokenJuice)](features/token-compression.md) compacts verbose tool output before it ever enters the model's context, so sweeping through your last six months of email costs single-digit dollars. [Automatic model routing](features/model-routing/) sends each task to the right model - `hint:reasoning` to a frontier model, `hint:fast` to a cheap one, vision to vision - all under one subscription. Optional [local AI via Ollama or LM Studio](features/model-routing/local-ai.md) keeps supported workloads on-device.
* [**Batteries included**](features/native-tools/)**.** A complete agent toolbelt is wired in by default: [web search](features/native-tools/web-search.md), a [web-fetch scraper](features/native-tools/web-scraper.md), a full [coder toolset](features/native-tools/coder.md) (filesystem, git, lint, test, grep), [browser & computer control](features/native-tools/browser-and-computer.md), [cron & scheduling](features/native-tools/cron.md), [memory tools](features/native-tools/memory-tools.md), [agent coordination](features/native-tools/agent-coordination.md) for spawning sub-agents, and [native voice](features/native-tools/voice.md) - STT in, TTS out, mascot lip-sync, and a live Google Meet agent that joins meetings, transcribes them into your Memory Tree, and can speak back into the call. No "install a plugin to read files" friction.
* [**Workflows**](features/workflows.md)**.** Durable, visual automations on the open-source tinyflows engine. Describe the automation in chat, the agent *proposes* a workflow graph, you review it on a canvas and save it. Flows fire on schedules or live app events, pause at approval gates, and resume exactly where they stopped, with full step-by-step run history.
* [**Meeting agents**](features/mascot/meeting-agents.md)**.** The mascot joins Google Meet, Zoom, Teams, and Webex as a real participant: animated face on the camera tile, its own voice in the call, a live transcript streaming into the app while the meeting happens. Connect your calendar (read-only) and it auto-joins on policy, wakes when addressed by name, and files summary + action items + transcript into a searchable history.
* [**A harness that finishes the job**](developing/architecture/agent-harness.md)**.** Every turn runs on the open-source tinyagents graph engine: durable checkpointing (sub-agents pause for your input and resume, instead of dying), a no-progress circuit breaker that stops identical-call loops and hands back a root-cause summary, classified tool failures rendered as actionable timeline cards, and durable, replayable run journals with per-call token and cost accounting.
* [**Privacy Mode**](features/privacy-mode.md)**.** One switch, enforced in the Rust core: local-only mode structurally blocks every cloud model call and permits only on-device runtimes (Ollama, LM Studio, MLX, local OpenAI-compatible).
* [**An agent economy**](features/tinyplace.md)**.** OpenHuman agents are citizens of tiny.place: a `@handle` identity, Signal-protocol E2E messaging with other agents, x402 USDC bounties and marketplace trading, all signed by an on-device wallet key that never touches disk.
* **Simple, UI-first.** A clean desktop experience and short onboarding paths take you from install to a working agent in a few clicks - no config-first setup, no terminal required. The agent has [a face](features/mascot/README.md): a desktop mascot that speaks, reacts to its surroundings, joins your meetings as a real participant, remembers you across weeks, and keeps thinking in the background even when you've stopped typing.

Together, these turn OpenHuman into something fundamentally different from a chatbot. It is an AI agent that consumes large amounts of personal data at low cost, maintains a persistent and evolving understanding of your world, and takes proactive actions on your behalf.

{% hint style="warning" %}
OpenHuman is not AGI. But it is a meaningful architectural step closer, with better memory, better orchestration, and better tooling.
{% endhint %}
