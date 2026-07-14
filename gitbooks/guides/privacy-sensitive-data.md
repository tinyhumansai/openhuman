---
description: >-
  A plain-language map of what OpenHuman keeps on your computer versus what it
  sends out, and the settings that let you keep sensitive work local.
icon: lock
---

# Keep sensitive data private

**Goal:** understand, in everyday language rather than architecture, what stays on your machine and what leaves it, so you can decide what OpenHuman should touch.

If you want the engineering detail, read [Privacy & Security](../features/privacy-and-security.md). This guide is the version you can act on in five minutes.

***

## The one-sentence version

**Your memory lives on your computer. The OpenHuman backend only handles the things that genuinely have to be brokered: signing you in, routing model requests, and talking to the services you connect.**

Everything below is an expansion of that sentence.

***

## What stays on your machine

These never leave your computer as raw data:

| Thing | Plain meaning |
| ----- | ------------- |
| **Your Memory Tree** | The database of everything OpenHuman has learned about your world. It's a file on your disk. |
| **Your Obsidian vault** | The human-readable Markdown version of that memory. Yours to read, edit, or delete. |
| **Audio you speak** | Captured to transcribe, then discarded. |
| **Local model state** | If you turn on [local AI](local-model.md), the model and its work stay on-device. |
| **Your persona and settings** | The files that define how your assistant behaves and what it's allowed to do. |

## What the backend handles (and why)

These leave your machine because they can't work otherwise, but note *what* is sent:

| Thing | What's actually sent |
| ----- | -------------------- |
| **Model requests** | Only what the assistant needs for that turn: your prompt plus the specific bits it pulled from your local memory. Not your whole memory, not background uploads. |
| **Web search** | Your search query goes to the backend proxy (so you don't need your own search key). |
| **Connected services** | When you connect Gmail, Slack, etc., the backend brokers each request. Your login tokens for those services are held by the backend, **not written in plain text on your laptop**. |
| **Text-to-speech** | The words to be spoken are streamed to generate audio, then discarded. They are not retained. |

{% hint style="info" %}
**Why local memory *is* the privacy design.** Most assistants trade privacy for context, because more context means more of your raw data uploaded. OpenHuman does the heavy work (chunking, scoring, summarizing) inside the local core, so the model only ever sees what you asked it to retrieve, at the moment you ask. Locality is the privacy feature, not a setting bolted on top.
{% endhint %}

## Two promises worth knowing

* **No training on your data.** Your conversations, memory, and personal information are never used to train models.
* **Secrets are stored by your operating system.** Local secrets are kept in your platform's secure store (macOS Keychain, Windows Credential Manager, or the Linux Secret Service), not lying around in app files. See [OS Keyring & Secret Storage](../features/os-keyring-and-secret-storage.md).

***

## Turning the dial toward "more local"

You have real controls. From most to least private:

1. **Route inference on-device.** Turn on [local AI with Ollama](local-model.md) so embeddings, summarization, and optionally chat/reasoning happen on your machine. *(Speech and web search still use the backend proxy even then.)*
2. **Tighten what the assistant can do.** In **Settings → Agents → Agent access**, set the autonomy tier. **Read-only** means it can observe and answer but never act or reach the network on its own. See the [Approval Gate](../features/approval-gate.md).
3. **Keep it in one folder.** By default the agent is confined to your workspace and cannot read the rest of your disk (`workspace_only` is on). System and credential folders (`~/.ssh`, `~/.gnupg`, `~/.aws`, and OS directories) are blocked outright, regardless of settings.
4. **Connect only what you need.** Every integration is a separate OAuth approval you grant (and can revoke) individually. Revoking stops the next sync; memory already collected stays local because it's yours.

## Built-in protections you didn't have to configure

* **Prompt-injection screening.** Incoming content is screened for attempts to hijack the assistant's instructions before it acts on them.
* **Secret & PII redaction on save.** When content is written into long-lived memory, OpenHuman strips things like API keys, tokens, private-key blocks, and personal identifiers so they don't get stored.
* **Encrypted in transit.** All traffic between the app and the backend is TLS. Nothing travels in plain text.

***

## Success checks

You know your privacy posture when you can answer these:

* [ ] Do you know your autonomy tier? (Check **Settings → Agents → Agent access**.)
* [ ] Do you know which integrations are connected? (Check **Settings**; disconnect any you don't need.)
* [ ] If locality matters for a workload, is [local AI](local-model.md) on and reporting `ready`?
* [ ] Are you comfortable that model turns send *retrieved snippets*, not your whole memory?

## Common misunderstandings

| Belief | Reality |
| ------ | ------- |
| "OpenHuman uploads my whole memory to answer." | It sends only what it retrieves for that specific turn. |
| "My service passwords are on my laptop." | Integration tokens are held by the backend; local secrets go in your OS keychain. |
| "Turning on local AI makes *everything* local." | Speech-to-text, text-to-speech, and web search still use the backend proxy by default. |
| "Revoking an integration deletes what it already gathered." | Already-ingested memory is yours and stays local; revoking only stops future syncing. |

## See also

* [Privacy & Security](../features/privacy-and-security.md): the detailed architecture.
* [Use OpenHuman with a local model](local-model.md): keep inference on-device.
* [Create a safe companion for a child](child-safe-companion.md): the strictest lockdown, composed from these controls.
