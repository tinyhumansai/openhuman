---
description: >-
  Carry your OpenHuman persona, memory, workspace, and model/provider config to
  a new computer, and understand which secrets travel and which you re-enter.
icon: truck
---

# Move OpenHuman to a new PC

**Goal:** set up OpenHuman on a new machine so it picks up where the old one left off (same memory, same persona, same settings), with credentials handled at a safe level of detail.

The short version: **copy one folder, sign back in.** The nuance is in what a folder copy does and does not carry, which this guide makes explicit so you're not surprised.

***

## Prerequisites

* Both computers available (or a backup of the old one's data folder).
* Your OpenHuman sign-in credentials.
* A way to move files between them (external drive, secure file transfer, etc.).

## What lives where

Everything OpenHuman persists is in a single folder:

| Platform | Data folder |
| -------- | ----------- |
| macOS / Linux | `~/.openhuman/` |
| Windows | `%USERPROFILE%\.openhuman\` |

Inside it, the things you care about migrating:

| What | Where (inside the data folder) | Travels with a folder copy? |
| ---- | ------------------------------ | --------------------------- |
| **Memory Tree** (the database) | `…/memory_tree/chunks.db` | ✅ Yes |
| **Obsidian vault** (readable memory) | `…/wiki/` | ✅ Yes |
| **Persona & behavior** | `SOUL.md`, `IDENTITY.md`, `HEARTBEAT.md` | ✅ Yes |
| **Config** (models, providers, routing, autonomy) | `config.toml` | ✅ Yes |
| **Session history** | `sessions/`, `session_raw/` | ✅ Yes |
| **Approval history** | `approval/approval.db` | ✅ Yes |
| **OS-stored secrets** (session token, some local keys) | Your OS keychain, **not** in this folder | ❌ No (re-established on sign-in) |
| **Integration access** (Gmail, Slack, …) | Brokered by the backend, tied to your account | ❌ No (reconnects on sign-in) |

{% hint style="info" %}
**Why some things don't travel, and why that's fine.** OpenHuman deliberately keeps secrets out of loose files. Your session token and certain local secrets live in the operating system's secure store (Keychain / Credential Manager / Secret Service), and your integration tokens are held by the backend against your account. So the folder copy carries your *data and persona*; **signing in on the new machine re-establishes the secrets and integrations.** You never hand-copy raw tokens between machines.
{% endhint %}

***

## Steps

### 1. Quit OpenHuman on the old machine

Fully close the app so nothing is mid-write to the database. A clean copy needs a quiet source.

### 2. Copy the data folder

Copy the **entire** data folder from the old machine to the same location on the new one:

* macOS / Linux: copy `~/.openhuman/` → `~/.openhuman/`
* Windows: copy `%USERPROFILE%\.openhuman\` → `%USERPROFILE%\.openhuman\`

Copy the whole folder rather than cherry-picking. It keeps memory, persona, config, and history consistent with each other.

The data folder holds config and memory but **not** the files the agent created or edited in its action sandbox. Also copy your **projects/action folder**, by default `~/OpenHuman/projects` (or wherever you pointed the action directory). Otherwise those project files stay behind on the old PC.

{% hint style="warning" %}
Copy it somewhere secure. This folder contains your personal memory in readable form. Treat the transfer like moving personal documents.
{% endhint %}

### 3. Install OpenHuman on the new machine

Install the current build from [tinyhumans.ai/openhuman](https://tinyhumans.ai/openhuman). If the data folder is already in place, the app will find it on launch. (Order doesn't strictly matter; installing first and copying after works too, as long as the app isn't running while you copy.)

### 4. Launch and sign in

Open the app and sign in with the **same account**. Signing in:

* Re-establishes your session token in the new machine's OS keychain.
* Reconnects your account so backend-brokered integrations come back.

### 5. Reconnect anything account-scoped

* **Integrations** (Gmail, Slack, etc.): confirm they show as connected under **Settings**. If any need a fresh OAuth approval, re-approve them. Each takes a quick click.
* **Bring-your-own keys:** if you had entered your own provider API key, a Composio direct key, or similar **local** secrets, re-enter them on the new machine. Those are stored in the OS keychain and don't come across in the folder.

### 6. Re-check model / provider config

Your `config.toml` came along, so model routing and provider choices should already match. If you used a [local model](local-model.md), remember that **Ollama/LM Studio is separate software**. Install it on the new machine too, and let OpenHuman re-pull the model weights (they aren't in the data folder).

***

## Success checks

The migration worked when:

* [ ] The **Memory** tab on the new machine shows your existing summaries. Your memory came across.
* [ ] The assistant replies in your configured style, and your display name/persona is intact.
* [ ] Connected integrations show as connected under **Settings** (reconnect any that don't).
* [ ] Your autonomy tier and settings match what you had (check **Settings → Agents → Agent access**).
* [ ] If you use local AI: Ollama is installed on the new machine and Local AI reports `ready` after models re-pull.

## Common failures

| Symptom | Cause | Fix |
| ------- | ----- | --- |
| New machine starts fresh, no memory | Data folder wasn't in the right place, or app was running during the copy | Quit the app, place the folder at `~/.openhuman/` (or `%USERPROFILE%\.openhuman\`), relaunch |
| Signed in but integrations are disconnected | Integration access is account/backend-scoped, not in the folder | Reconnect each integration in Settings (one OAuth click each) |
| Local model doesn't work on the new PC | Ollama/LM Studio and the weights aren't on the new machine | Install the runtime and let models re-pull; see [local model guide](local-model.md) |
| Assistant lost its personality | `SOUL.md` / `IDENTITY.md` weren't copied | Copy the **whole** data folder, not just the database |
| Sign-in stalls on the new machine | An auth/handler issue unrelated to migration | See [Troubleshooting Sign-In](../overview/troubleshooting-sign-in.md) |

## Recovery

* **Keep the old machine's folder until you've verified the new one.** Don't wipe the source until every success check passes.
* If the new machine won't start at all, treat it as a fresh-install problem: [Recover from a failed installation](recover-failed-installation.md). Your copied folder is safe to move aside and restore.

## See also

* [Recover from a failed installation](recover-failed-installation.md): same data folder, different problem.
* [Keep sensitive data private](privacy-sensitive-data.md): why secrets are stored the way they are.
