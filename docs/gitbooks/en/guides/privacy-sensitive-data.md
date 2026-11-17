---
description: >-
  A plain-language map of what OpenHuman keeps on your computer and what it
  sends out, and the settings that keep sensitive work local.
icon: lock
---

# Keep sensitive data private

This guide explains in everyday language what stays on your machine and what leaves it, so you can decide what OpenHuman should touch. For the engineering detail, read [Privacy and security](../features/privacy-and-security.md). This page is the version you can act on in five minutes.

## The short version

Your settings and local secrets stay on your computer. Your memory is stored in CortexDB, either hosted by TinyHumans for your account or your own. The OpenHuman backend handles what has to be brokered: signing you in, hosted memory, routing model requests, and talking to the services you connect.

## What stays on your machine

These never leave your computer as raw data:

| Thing | Plain meaning |
| --- | --- |
| Audio you speak | Captured to transcribe, then discarded. |
| Local model state | If you use [local AI](local-model.md), your runtime and its models stay on-device. |
| Your persona and settings | The files that define how your assistant behaves and what it may do. |

## Where your memory is stored

When memory is on, documents, conversations, learnings and the beliefs built from them are stored in CortexDB. With TinyHumans-hosted memory they are not stored on your computer. A CortexDB you host yourself can keep them on your own machine. With no engine available, memory is off and nothing is stored.

- **TinyHumans engine (the default when signed in).** Items are stored in the hosted CortexDB that TinyHumans runs, through the TinyHumans backend. Each account gets its own isolated tenant, so other users cannot see your memory.
- **CortexDB engine.** Items are stored in your own CortexDB account or self-hosted CortexDB, reached directly with your key. A CortexDB on your own machine keeps the data on that machine.

Secrets and personal identifiers are scrubbed before an item is sent. You can forget single items or whole sources from the Memory page. See [Deleting memory](../features/privacy-and-security.md#deleting-memory) for erasing everything.

## What the backend handles

These leave your machine because they cannot work otherwise. Here is what is sent:

| Thing | What is sent |
| --- | --- |
| Hosted memory | With the TinyHumans engine, every item stored and every recall goes through the backend to hosted CortexDB. |
| Model requests | Only what the assistant needs for that turn: your prompt plus the memory recalled for it. Not your whole memory. |
| Web search | Your search query goes to the backend proxy, so you do not need your own search key. |
| Connected services | When you connect Gmail, Slack and so on, the backend brokers each request. It holds your login tokens for those services, so they are not written in plain text on your laptop. |
| Text-to-speech | The words to be spoken are streamed to generate audio, then discarded. They are not retained. |

{% hint style="info" %}
Memory is not local, so scrubbing and scoping matter. Items are scrubbed before they are sent, tool-call arguments are never stored, hosted memory is isolated per account, and a model turn sees only what was recalled for it.
{% endhint %}

## Two promises

- **No training on your data.** Your conversations, memory and personal information are never used to train models.
- **Secrets live in your operating system's store.** Local secrets are kept in macOS Keychain, Windows Credential Manager or the Linux Secret Service, not in app files. See [OS keyring and secret storage](../features/os-keyring-and-secret-storage.md).

## Making it more local

You have real controls. These run from most to least private:

1. **Route inference on-device.** Run a local runtime such as Ollama yourself, pull the models, and [add it as a provider](local-model.md). Embeddings, summaries and, optionally, chat and reasoning then happen on your machine. OpenHuman does not install the runtime or download models for you. Speech and web search still use the backend proxy.
2. **Tighten what the assistant can do.** Set `[autonomy] enabled = true` and `level = "readonly"` in `config.toml`. The assistant can then observe and answer but never act or reach the network on its own. The policy is off by default, so you have to turn this on. See the [Approval Gate](../features/approval-gate.md).
3. **Keep it in one folder.** The filesystem boundary needs two things: the policy enabled and `workspace_only` on. With both, the agent is confined to its working folder and cannot read the rest of your disk. If either is off, the boundary is not enforced. A trusted root is the deliberate exception. Each one you add grants its subtree outside the working folder and takes precedence over `workspace_only`. System and credential folders (`~/.ssh`, `~/.gnupg`, `~/.aws` and OS directories) are always blocked.
4. **Connect only what you need.** Each integration is a separate OAuth approval that you grant and can revoke. Revoking stops the next sync. Memory already collected stays in your memory engine until you forget it.

## Built-in protections

- **Prompt-injection screening.** Incoming content is screened for attempts to hijack the assistant's instructions before it acts on them.
- **Secret and personal-data redaction on save.** When content is written into long-lived memory, OpenHuman strips API keys, tokens, private-key blocks and personal identifiers.
- **Encryption in transit.** All traffic between the app and the backend uses TLS.

## Success checks

You know your privacy posture when you can answer these:

- [ ] Is the autonomy policy on, and do you know its tier? Check `[autonomy]` in `config.toml`.
- [ ] If you rely on the filesystem boundary, is `workspace_only` on too, and do you know which trusted roots grant access outside the working folder?
- [ ] Do you know which integrations are connected? Check **Settings** and disconnect any you do not need.
- [ ] If a workload must stay local, is it routed to a [local provider](local-model.md) that is running and answering?
- [ ] Are you comfortable that model turns send retrieved snippets, not your whole memory?

## Common misunderstandings

| Belief | Reality |
| --- | --- |
| "My memory is stored on my laptop." | It is stored in CortexDB, hosted by TinyHumans or run by you. Only bookkeeping is local. |
| "OpenHuman uploads my whole memory to the model to answer." | The model gets only what was recalled for that turn. |
| "My service passwords are on my laptop." | The backend holds integration tokens, and local secrets go in your OS keychain. |
| "Turning on local AI makes everything local." | Speech-to-text, text-to-speech and web search still use the backend proxy by default, and memory is still stored in CortexDB. |
| "Revoking an integration deletes what it already gathered." | Memory already ingested stays in your memory engine until you forget it. Revoking only stops future syncing. |

## See also

- [Privacy and security](../features/privacy-and-security.md): the detailed architecture.
- [Use OpenHuman with a local model](local-model.md): keep inference on-device.
- [Create a safe companion for a child](child-safe-companion.md): the strictest lockdown, built from these controls.
