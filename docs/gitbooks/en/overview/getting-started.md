---
description: >-
  Install OpenHuman, sign in, choose how AI runs, connect an app, and make your
  first request against your own memory.
icon: play
---

# Getting started

This page takes you from install to your first request. OpenHuman is open source under the GNU GPL3 license, and the code is at [github.com/tinyhumansai/openhuman](https://github.com/tinyhumansai/openhuman).

{% hint style="info" %}
If you want a specific outcome, such as a private assistant, a local model, a broken install fixed, or a move to a new machine, the [Guides](../guides/README.md) have task-by-task walkthroughs.
{% endhint %}

## System requirements

OpenHuman runs on macOS, Windows and Linux desktops. 4 GB of RAM is recommended. Plan for 16 GB or more if you will ingest very large mailboxes or repos, or run a [local model](../features/model-routing/local-ai.md) on the same machine. For a local model you install the runtime (such as Ollama) and pull the models yourself.

The first time you launch the app, your OS asks for the permissions it needs, such as Accessibility on macOS when you use desktop control. You can review them any time under **Settings**.

## 1. Download and install

Download the desktop app from [tinyhumans.ai/openhuman](https://tinyhumans.ai/openhuman) or the [latest GitHub release](https://github.com/tinyhumansai/openhuman/releases/latest). You can also use the install scripts, which download the release and check its SHA-256 before installing:

```bash
# macOS and Linux
curl -fsSL https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.sh | bash
```

```powershell
# Windows (PowerShell)
irm https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.ps1 | iex
```

A Homebrew cask also exists, but it can lag the latest release. Open the app once it is installed.

## 2. Sign in

The first screen is **Sign in! Let's Cook**. You can sign in with social login or other options. An **Advanced** panel lets you point the app at a custom core RPC URL if you run your own backend. Most people can ignore it.

{% hint style="info" %}
Signing in does not give OpenHuman ongoing access to anything. Each third-party integration needs its own OAuth approval in the steps below.
{% endhint %}

{% hint style="warning" %}
Your workspace config and runtime state live on your machine. The default setup still uses OpenHuman-hosted services for sign-in, model routing, managed integration OAuth and tool calls, and web search. Use the custom setup paths to bring your own model, search or Composio credentials. Some hosted features and real-time integration triggers need the managed backend.
{% endhint %}

## 3. Connect something and set up memory

Onboarding asks how AI should run. You can use the managed TinyHumans route, or a custom setup where you bring your own model, search, embeddings and connector keys. Then it drops you into the app. Two things make your first request worth making:

- **An integration.** Open [Connections](../features/connections.md), then Apps, and connect one. Gmail is a common first choice. Each connection is a one-click OAuth approval that you can revoke.
- **A memory engine.** Open Connections, then Memory. When you are signed in, the hosted engine is already there. Otherwise point it at your own CortexDB. With neither, [memory](../features/memory.md) is off and the agent starts from zero every chat.

Add the integration as a memory source on the Brain tab to sync it on a schedule.

## 4. Run your first request

Once a memory source has synced, try prompts like these.

Briefings:

- "What do I need to know from the last 12 hours?"
- "What's waiting on me?"

Cross-source questions:

- "Summarize what I missed today."
- "What are the key decisions from this week?"
- "Extract action items from my recent conversations."
- "What did Sarah say about the project across email and chat?"

OpenHuman picks the right model for each task. See [Automatic model routing](../features/model-routing/README.md).

## 5. Keep going

Now that the agent has memory and a model, the rest of the product gives it more to work with:

- [Chat](../features/chat.md): what a turn can do, and the four ways it stops to ask you something.
- [Connections](../features/connections.md): everything the agent plugs into, on one page.
- [Memory](../features/memory.md): connect more sources and sync them on a schedule.
- [Workflows](../features/workflows.md): describe an automation and review the graph the agent proposes.
- [Native voice](../features/native-tools/voice.md): dictation, spoken replies, or a live voice agent.
- [Roadmap](roadmap.md): what is coming, and what was removed.

## Join the community

OpenHuman is in early beta, and feedback and contributions matter.

- GitHub: [github.com/tinyhumansai/openhuman](https://github.com/tinyhumansai/openhuman)
- Discord: [guild.tinyhumans.ai](https://guild.tinyhumans.ai)
