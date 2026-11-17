---
description: >-
  Go from a fresh install to a personal assistant that knows your context,
  respects the boundaries you set, and acts only with your approval at the
  supervised tier.
icon: robot
---

# Create my personal AI assistant

This is the start-here guide. By the end you have an assistant that remembers part of your world, replies in a style you like, and, on the supervised tier, takes no real-world action without your say-so. It assumes nothing beyond a downloaded app.

## Prerequisites

- OpenHuman installed on macOS, Windows, or Linux. If you have not installed yet, do [Getting started](../overview/getting-started.md) first, then come back.
- 4 GB+ RAM (16 GB or more if you plan to connect very large mailboxes or run a [local model](local-model.md)).
- An account to sign in with (social login works).
- Optional: one account you'd like the assistant to know about (Gmail is the usual first one).

## Privacy implications

- Signing in does not grant ongoing access to anything. Every integration is a separate OAuth approval that you can revoke later.
- Your memory is stored by the engine you choose: hosted TinyHumans or your own CortexDB. With neither, memory is off.
- By default, chat and reasoning run through the OpenHuman-hosted [model router](../features/model-routing/README.md). If you want inference on-device instead, see [Use OpenHuman with a local model](local-model.md).
- For the full picture, see [Keep sensitive data private](privacy-sensitive-data.md).

## Steps

### 1. Sign in

Launch the app. The first screen is **Sign in! Let's Cook**. Choose a login option. An **Advanced** panel lets you point at a custom core, which most people can ignore.

### 2. Choose how AI runs

After sign-in you choose how AI runs:

- **Cloud** takes one click, and the hosted model router handles inference. It is the fastest way to a working assistant.
- **Custom** lets you choose your inference provider, voice, integrations (OAuth), web search and embeddings yourself.

If you are not sure, pick Cloud. You can change any of this later in **Settings**.

### 3. Give it something to remember

An assistant with no memory is a chatbot. Connect at least one source so it has context:

- Open **Settings** and connect an integration (Gmail is the common starting point). Each connection is a one-click OAuth approval.
- Add folders, files or links as sources under **Connections → Memory** so they sync into [Memory](../features/memory.md) on a schedule.

### 4. Set your boundaries

Open `config.toml` in your data folder (`~/.openhuman/config.toml`, or `%USERPROFILE%\.openhuman\config.toml` on Windows) and turn the policy on:

```toml
[autonomy]
enabled = true
level = "supervised"   # "readonly" | "supervised" | "full"
```

| Tier | What it means |
| --- | --- |
| `readonly` | The assistant can observe and answer, but never acts: no sending, no file writes, no commands. |
| `supervised` | It can act, but any state-changing, network, install or destructive action waits for your approval first, unless its tool is on your always-allow list. |
| `full` | Routine actions run automatically. Network, install and destructive actions still ask. |

{% hint style="warning" %}
The policy is off until you set `enabled = true`. There is no user-facing switch for the tier, because the panel that holds the radios is a developer-only deep link. Until you turn the policy on, acting tool calls run without a prompt. Credential stores (`~/.ssh`, `~/.gnupg`, `~/.aws`) and system roots stay blocked either way.
{% endhint %}

`supervised` is the right starting point. With it on, nothing with an external effect happens in a chat unless you say yes. That check is the [Approval Gate](../features/approval-gate.md). The one exception is a tool you add to the always-allow list by answering **Always allow** to a prompt. That tool then runs without a fresh approval. Remove it from the list in **Settings → Agent access** to get per-call review back.

Once the policy is on, **Settings → Agent access** holds the rest of the boundary: the trusted roots, the always-allow list, the action timeout and the workspace-only switch.

### 5. Shape its personality (optional)

An editable prompt called `SOUL.md` defines how the assistant talks and behaves. Its mission and values live in a companion file, `IDENTITY.md`. Both ship with sensible defaults, so you do not have to touch them, but you can:

- Set a display name and short description in **Settings → Personality**.
- Edit the behavior on the **Brain** page (the raised center button in the bottom bar, `/brain`), where memory, goals and intelligence live.

OpenHuman also learns lasting preferences from how you correct it. See [Personalization and self-learning](../features/personalization.md).

### 6. Run your first real request

Once a source has synced, try:

- "What do I need to know from the last 12 hours?"
- "What's waiting on me?"
- "Summarize what I missed today."

## Success checks

You have a working assistant when all of these are true:

- [ ] The app is signed in (you are past the welcome screen and in the chat or home view).
- [ ] At least one integration shows as connected in Settings.
- [ ] A briefing prompt ("what's waiting on me?") returns something drawn from your actual data, not a generic answer.
- [ ] Connections → Memory → Brain shows your source with an item count after it syncs.
- [ ] When you ask it to do something with an external effect (for example "draft and send an email"), an Approval Request card appears above the chat box instead of the assistant acting silently.

## Common failures

| Symptom                                          | What it means                                                          | Fix                                                                                               |
| ------------------------------------------------ | ---------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| Sign-in returns to the welcome screen            | The OAuth callback did not reach the app                                | Follow [Troubleshooting sign-in](../overview/troubleshooting-sign-in.md)                          |
| Connected a source but memory stays empty        | Source not added or not synced yet, or the OAuth scope is too narrow    | Press sync on the source; re-check the connection in Settings                                     |
| Assistant answers generically, ignores your data | It answered without recalling memory                                   | Ask again and name the source ("from my email"). Confirm the source is connected |
| It did something you did not expect         | The autonomy policy is off, or `level` is `full`                       | Set `[autonomy] enabled = true` and `level = "supervised"` in `config.toml`                       |

## Recovery

- **Reset boundaries fast.** If the assistant is doing too much, set both `enabled = true` and `level = "readonly"` in `config.toml`. The level alone changes nothing while `enabled` is `false`, because the whole policy is off then. With both set, it takes effect on the next turn and blocks all acting.
- **Nothing you connect is permanent.** Revoke any integration from Settings. Items already in your memory engine stay until you forget them, or until you tick **Also delete memory from this source** when disconnecting. The next sync stops pulling that source.
- **If the app will not start,** see [Recover from a failed installation](recover-failed-installation.md). Your configuration is preserved by default.

## Next steps

- [Use OpenHuman with a local model](local-model.md): keep inference on-device.
- [Keep sensitive data private](privacy-sensitive-data.md): see what leaves your machine.
