---
description: >-
  Go from a fresh install to a working personal assistant that knows your
  context, respects your boundaries, and acts only with your approval.
icon: robot
---

# Create my personal AI assistant

**Goal:** a working assistant that has some memory of your world, replies in a style you like, and never takes a real-world action without your say-so.

This is the "start here" guide. It assumes nothing beyond a downloaded app.

***

## Prerequisites

* OpenHuman installed on macOS, Windows, or Linux. If you haven't installed yet, do [Getting Started](../overview/getting-started.md) first, then come back.
* 4 GB+ RAM (16 GB+ if you plan to connect very large mailboxes or run a [local model](local-model.md)).
* An account to sign in with (social login works).
* Optional: one account you'd like the assistant to know about (Gmail is the usual first one).

## Privacy implications

* Signing in **does not** grant ongoing access to anything. Every integration is a separate, explicit OAuth approval you can revoke later.
* Your memory (the local database and the Markdown vault) is created **on your machine**. Raw source data does not sit on the OpenHuman backend.
* By default, chat/reasoning runs through the OpenHuman-hosted [model router](../features/model-routing/). If you want inference on-device instead, see [Use OpenHuman with a local model](local-model.md).
* Full detail: [Keep sensitive data private](privacy-sensitive-data.md).

***

## Steps

### 1. Sign in

Launch the app. The first screen is **"Sign in! Let's Cook"**. Choose a login option. There is an **Advanced** panel for pointing at a custom core; most people ignore it.

### 2. Choose how AI runs

After sign-in you'll hit a **runtime choice**:

* **Cloud**: one click, and the hosted model router handles inference. This is the fastest path to a working assistant.
* **Custom**: walk through choosing your inference provider, voice, integrations (OAuth), web search, and embeddings yourself.

If you're not sure, pick **Cloud**. You can change any of this later in **Settings**.

### 3. Give it something to remember

An assistant with no memory is just a chatbot. Connect at least one source so it has context to draw on:

* Open **Settings** and connect an integration (Gmail is the common starting point). Each connection is a one-click OAuth approval.
* Once connected, [auto-fetch](../features/obsidian-wiki/auto-fetch.md) starts pulling data into your [Memory Tree](../features/obsidian-wiki/memory-tree.md) on a schedule (the first Gmail tick lands within about twenty minutes).

### 4. Set your boundaries

Open **Settings → Agents → Agent access**. This controls how much the assistant can do on its own:

| Tier | What it means |
| ---- | ------------- |
| **Read-only** | The assistant can observe and answer, but never acts (no sending, no file writes, no commands). |
| **Supervised** *(default)* | It can act, but any state-changing, network, install, or destructive action is **parked for your approval** first. |
| **Full** | Routine actions run automatically; network/install/destructive actions still ask. |

Leave it on **Supervised** unless you have a reason not to. Nothing with an external effect will happen in a chat without you saying yes. This is the [Approval Gate](../features/approval-gate.md), and it's on by default.

### 5. Shape its personality (optional)

How the assistant talks and behaves is defined by an editable prompt called **`SOUL.md`** (its mission and values live in a companion **`IDENTITY.md`**). You don't have to touch these (they ship with sensible defaults), but you can:

* Set a display name and short description in **Settings → Personality**.
* Edit the behavior itself via the **Brain** page (the raised center button in the bottom bar, `/brain`), where memory, goals, and intelligence live.

OpenHuman also **learns** durable preferences from how you correct it over time. See [Personalization & Self-Learning](../features/personalization.md).

### 6. Run your first real request

Once a source has been ingested, try:

* "What do I need to know from the last 12 hours?"
* "What's waiting on me?"
* "Summarize what I missed today."

***

## Success checks

You have a working assistant when **all** of these are true:

* [ ] The app is signed in (you're past the welcome screen and in the chat/home view).
* [ ] At least one integration shows as **connected** in Settings.
* [ ] A briefing prompt ("what's waiting on me?") returns something drawn from your actual data, not a generic answer.
* [ ] Opening the **Memory** tab shows summaries appearing (give it one auto-fetch cycle, up to about 20 minutes, after connecting a source).
* [ ] When you ask it to do something with an external effect (e.g. "draft and send an email"), you see an **Approval Request card** appear above the chat box rather than it silently acting.

## Common failures

| Symptom | What it means | Fix |
| ------- | ------------- | --- |
| Sign-in returns to the welcome screen | The OAuth callback didn't reach the app | Follow [Troubleshooting Sign-In](../overview/troubleshooting-sign-in.md) |
| Connected a source but memory stays empty | First auto-fetch tick hasn't run yet, or the OAuth scope is too narrow | Wait one cycle (~20 min); re-check the connection in Settings |
| Assistant answers generically, ignores your data | It answered without recalling memory | Ask again and reference the source explicitly ("from my email…"); confirm the source is connected |
| It performed an action you didn't expect | Autonomy tier may be set to **Full** | Set **Settings → Agents → Agent access** back to **Supervised** |

## Recovery

* **Reset boundaries fast:** if the assistant is doing too much, drop the tier to **Read-only** in Agent access. It takes effect on the next turn and blocks all acting immediately.
* **Nothing you connect is permanent:** revoke any integration from Settings; chunks already in your local memory stay (they're yours), and the next sync tick stops pulling that source.
* **If the app itself won't start,** see [Recover from a failed installation](recover-failed-installation.md). Your configuration is preserved by default.

***

## Next steps

* [Use OpenHuman with a local model](local-model.md): keep inference on-device.
* [Connect OpenHuman to Obsidian](connect-obsidian.md): read and edit the memory by hand.
* [Keep sensitive data private](privacy-sensitive-data.md): understand exactly what leaves your machine.
