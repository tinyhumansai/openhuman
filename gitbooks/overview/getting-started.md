---
description: >-
  Install OpenHuman, walk through the in-app onboarding (sign in, connect Gmail,
  choose how AI runs), and run your first request against your own memory.
icon: play
---

# Getting Started

This page walks you through installing OpenHuman, going through the in-app onboarding, and running your first request.

OpenHuman is open source under the GNU GPL3 license. The codebase is at [github.com/tinyhumansai/openhuman](https://github.com/tinyhumansai/openhuman).

{% hint style="info" %}
**Want a specific outcome?** If you're here to accomplish something concrete (set up a private assistant, run a local model, recover a broken install, or move to a new machine), the [Guides](../guides/README.md) section has task-by-task walkthroughs.
{% endhint %}

---

## System requirements

OpenHuman runs on **macOS, Windows and Linux** desktops. 4 GB+ RAM is recommended; 16 GB+ if you intend to ingest very large mailboxes or repos, or run a [local model](../features/model-routing/local-ai.md) on the same machine (you install the runtime, such as Ollama, and pull the models yourself).

### Permissions

The first time you launch OpenHuman, the OS will prompt for the permissions the app needs (Accessibility on macOS, Input Monitoring for the voice hotkey). You can review and adjust these any time under **Settings**.

---

## 1. Download and install

Get the OpenHuman desktop app from [http://tinyhumans.ai/openhuman](http://tinyhumans.ai/openhuman) or via your platform's package manager. Open the app once it's installed.

## 2. Sign in

The first screen is **"Sign in! Let's Cook"**. Multiple sign-in options are available, including social login. There's also an **Advanced** panel for pointing the app at a custom core RPC URL if you're running your own backend; most users can ignore it.

{% hint style="info" %}
**No permanent lock-in.** Signing in does not grant OpenHuman ongoing access to anything. All third-party access requires explicit OAuth approval per integration in the steps below.
{% endhint %}

{% hint style="warning" %}
**Know what is local and what is managed.** Your workspace config, and local runtime state live on your machine. The default setup still uses OpenHuman-hosted services for sign-in, model routing, managed integration OAuth/tool calls, and web search proxying. Use the custom setup paths if you want to bring your own model, search, or Composio credentials. Some hosted features and real-time integration triggers still require the managed backend.
{% endhint %}

## 3. Run your first request

Once Gmail is connected and a memory source has synced, try prompts like:

**Briefings**

- "What do I need to know from the last 12 hours?"
- "What's waiting on me?"

**Cross-source queries**

- "Summarize what I missed today."
- "What are the key decisions from this week?"
- "Extract action items from my recent conversations."
- "What did Sarah say about the project across email and chat?"

OpenHuman picks the right model for each task automatically. See [Automatic Model Routing](../features/model-routing/).

---

## 4. Set up memory

Open **Connections → Memory**. On the **Engine** tab choose TinyHumans (signed in) or your own CortexDB, then add a folder, link or GitHub repo on the **Documents** tab. The agent can then answer questions about it, with citations. See [Memory](../features/memory.md).

---

## 5. Let the mascot do more

Now that the agent has memory and a model, the rest of the product is about giving it more surfaces:

- [**Memory**](../features/memory.md) - connect more sources; they sync on a schedule into your memory engine.
- [**Native Voice**](../features/native-tools/voice.md) - push-to-talk dictation and TTS replies so you can talk to OpenHuman instead of typing.

## Join the community

OpenHuman is in early beta. Feedback and contributions make a real difference at this stage.

- **GitHub:** [github.com/tinyhumansai/openhuman](https://github.com/tinyhumansai/openhuman)
- **Discord:** [guild.tinyhumans.ai](https://guild.tinyhumans.ai)
