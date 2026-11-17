---
description: >-
  An open-source AI assistant for your desktop with a persistent memory of your
  life, agent fleets and workflows, and a Rust core small enough to embed.
icon: diamond
---

# Welcome to OpenHuman

<figure><img src=".gitbook/assets/demo.png" alt=""><figcaption></figcaption></figure>

OpenHuman is an open-source AI assistant for your desktop. It is built to be three things most assistants are not. It is a brain, with a persistent, local, readable memory of your world. It is an orchestrator, with durable agent graphs, visual workflows, sub-agent fleets and [end-to-end encrypted agent-to-agent sessions](features/orchestration.md). And it is a deep researcher, sweeping your data and the web before you finish asking.

It runs on a Rust core inside a Tauri desktop shell and is licensed under GNU GPL3. The source is at [github.com/tinyhumansai/openhuman](https://github.com/tinyhumansai/openhuman).

Most language models are stateless. You send a prompt, get a reply, and the context is gone. Even assistants with "memory" keep a few bullet points, which is a sticky note and not understanding. OpenHuman takes a different approach.

## What you get

- [**Memory you can inspect.**](features/memory.md) Documents (folders, files, links, GitHub, RSS, connected apps), conversations and learnings go into a memory engine, either hosted by TinyHumans or your own CortexDB. The agent answers from it with citations, and a short brief about you opens every new chat. You can see and delete everything stored.
- [**Third-party integrations.**](features/integrations/README.md) One-click OAuth into Gmail, GitHub, Slack, Notion, Stripe, Calendar, Drive, Linear, Jira and more, across 119 managed toolkits. You do not wire API keys by hand.
- **An agent built for big data.** [Token compression](features/token-compression.md) shrinks verbose tool output before it reaches the model, so sweeping through six months of email costs single-digit dollars. [Automatic model routing](features/model-routing/README.md) sends each task to the right model under one subscription and one TinyHumans API key. Optional [local AI through Ollama or LM Studio](features/model-routing/local-ai.md) keeps supported work on your device. Every model, embedding, memory and web-search engine is swappable through config.
- **Fast, cheap decisions.** Before an agent spends a full model call choosing between dozens of tools, a small classifier called Jev scores the options in about 150 milliseconds. It shortlists tools, decides whether any tool is needed, and drives routine browser steps. The expensive model runs only when a task needs it.
- [**Batteries included.**](features/native-tools/README.md) The default toolbelt has [web search](features/native-tools/web-search.md), a [web scraper](features/native-tools/web-scraper.md), a [coder toolset](features/native-tools/coder.md) (filesystem, git, lint, test, grep), [browser and computer control](features/native-tools/browser-and-computer.md), [cron and scheduling](features/native-tools/cron.md), [memory tools](features/native-tools/memory-tools.md), [agent coordination](features/native-tools/agent-coordination.md) for sub-agents, and [native voice](features/native-tools/voice.md) with speech in, speech out and a live voice agent you can interrupt.
- [**Workflows.**](features/workflows.md) Describe an automation in chat. The agent proposes a workflow graph, you review it on a canvas and save it. Flows run on schedules or app events, pause at approval gates, resume where they stopped, and keep a step-by-step run history.
- [**A harness that finishes the job.**](developing/architecture/agent-harness.md) Every turn is checkpointed, so sub-agents pause for your input and resume instead of dying. A circuit breaker stops loops of identical calls and hands back a root-cause summary. Tool failures show up as actionable cards, and every run has a replayable journal with per-call token and cost accounting.
- **Light enough to run in-process.** The Rust core runs inside the desktop app rather than as background services. We measured 500 agents alive in one process at about 1.77 MiB of marginal memory each. The same core ships as a library ([`crates/openhuman-embed`](https://github.com/tinyhumansai/openhuman/tree/main/crates/openhuman-embed), with a [one-key setup](developing/tinyhumans-api-key.md)), so another product can boot a runtime and add agents in a few lines of Rust. See [Performance](developing/performance.md), the public [benchmarks repo](https://github.com/tinyhumansai/openhuman-benchmarks), [Pluggable engines](developing/engines.md) and [Embedding OpenHuman](developing/embedding.md).
- [**Privacy Mode.**](features/privacy-mode.md) One switch, enforced in the Rust core. Local-only mode blocks every cloud model call and allows only on-device runtimes (Ollama, LM Studio, MLX, or a local OpenAI-compatible server).
- **Simple and UI-first.** Short onboarding takes you from install to a working agent in a few clicks, with no terminal needed. The agent has [a face](features/mascot/README.md): a desktop mascot that speaks, reacts to its surroundings, remembers you across weeks and keeps thinking in the background.

Together, these make OpenHuman more than a chatbot. It takes in large amounts of personal data at low cost, keeps an evolving understanding of your world, and acts on your behalf.

{% hint style="warning" %}
OpenHuman is not AGI. It is a step in that direction, with better memory, orchestration and tooling.
{% endhint %}

## For developers

Start with [Getting set up](developing/getting-set-up.md), then [Building the Rust core](developing/building-rust-core.md) and the [Architecture](developing/architecture/README.md) overview. To use the core from your own Rust product, read [Embedding OpenHuman](developing/embedding.md) and [One TinyHumans API key](developing/tinyhumans-api-key.md). [Loadable modules](developing/loadable-modules.md) covers the native module system, and [Performance and footprint](developing/performance.md) has the numbers. The benchmark rig and results are public at [tinyhumansai/openhuman-benchmarks](https://github.com/tinyhumansai/openhuman-benchmarks).
