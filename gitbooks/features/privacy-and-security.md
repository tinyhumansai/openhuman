---
icon: shield
---

# Privacy & Security

OpenHuman is designed so that the **memory of your life lives on your machine**. The local SQLite Memory Tree, the Markdown Obsidian vault, your audio buffers, all of that stays under your control. The OpenHuman backend handles things that have to be brokered (LLM calls, OAuth tokens, search proxying), and nothing more.

***

## Privacy by Design

**The Memory Tree is local.** The SQLite database (`<workspace>/memory_tree/chunks.db`) and the Markdown vault (`<workspace>/wiki/`) live on your machine. The agent reads from them locally; nothing about your raw source data sits on the OpenHuman backend.

**Integration tokens are held by the backend, not on your laptop.** OAuth tokens are never written to disk in plaintext on your device. The OpenHuman backend brokers each integration request, the core never speaks any third-party API directly.

**OS-level credential storage.** Sensitive local secrets are rooted in your platform's secure keychain, macOS Keychain, Windows Credential Manager, Linux Secret Service. See [OS Keyring & Secret Storage](os-keyring-and-secret-storage.md).

**No training on your data.** Your conversations, your Memory Tree, and your personal information are never used to train AI models or improve systems.

**Optional** [**Local AI**](model-routing/local-ai.md)**.** If you want embeddings and summary-tree building to stay on your machine, opt in. Heartbeat / learning / subconscious loops can be moved on-device the same way.

***

## What stays on your machine

|                                 |                                                                 |
| ------------------------------- | --------------------------------------------------------------- |
| **Memory Tree SQLite database** | Local - `<workspace>/memory_tree/chunks.db`.                    |
| **Obsidian Markdown vault**     | Local - `<workspace>/wiki/`. Yours to read, edit, copy, delete. |
| **Audio capture buffers**       | Local. Discarded after STT.                                     |
| **Local model state**           | Local.                                                          |

## What the OpenHuman backend handles

|                                    |                                                                                                                                                                            |
| ---------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **LLM calls**                      | Proxied through the backend under one subscription, then forwarded to the underlying provider (Anthropic / OpenAI / Google / etc.) per the [model router](model-routing/). |
| **Web search proxy**               | The native [web search tool](native-tools/web-search.md) uses the backend proxy by default so you don't carry a search API key. If you call the optional SearXNG tool, that query goes to your configured SearXNG instance instead. |
| **Integration OAuth & tool proxy** | Token storage and rate-limited request brokering for [118+ integrations](integrations/README.md).                                                                                 |
| **TTS streaming**                  | Hosted [text-to-speech](native-tools/voice.md) audio streams. Audio is generated and discarded - not retained.                                                                          |

***

## Permissions and access control

OpenHuman accesses an integration only after you complete its OAuth flow. Each connection has its own scope; you can revoke any of them at any time from the Skills tab.

[Auto-fetch](obsidian-wiki/auto-fetch.md) does run continuously while a connection is active, that is the whole point. But it is bound by:

* The **OAuth scope** you granted that integration.
* A **per-provider sync interval** (e.g. Gmail every 15 min by default).
* A **daily budget** per connection that caps API usage.

If you revoke a connection, the next tick stops syncing it; chunks already in your local Memory Tree remain there because they're yours.

***

## Why a local memory is privacy

Most AI assistants face a tradeoff: more context means more raw data sent to the cloud. The Memory Tree eliminates this tradeoff.

Because canonicalization, chunking, scoring and summary trees all run **inside your local Rust core**, your raw source data never leaves your machine. The only thing the LLM sees is what the agent retrieves from your local Memory Tree at the moment of a turn, and that retrieval is governed by your prompt, not by background uploads.

Compression and locality together become the privacy architecture.

<figure><img src="../.gitbook/assets/V17 — Privacy Shield@2x.png" alt=""><figcaption></figcaption></figure>

## Security

**Encrypted in transit.** All communication between the application and the OpenHuman backend uses TLS. No data travels in plain text.

**Key in keyring, ciphertext on disk.** For local secrets that must be persisted in app files, OpenHuman stores encrypted ciphertext on disk and keeps the master decryption key in the OS keyring. See [OS Keyring & Secret Storage](os-keyring-and-secret-storage.md).

**Sandboxed skills.** Each skill runs in its own isolated execution environment with enforced memory and resource limits. Skills cannot access each other's data, the host system's file system, or your credentials.

**Workspace-scoped tools.** The native [filesystem tools](native-tools/coder.md) operate within the workspace the user opens; they do not have ambient access to the rest of the disk.

**Short-lived tokens.** Authentication tokens between the app and the backend are time-limited.

***

## Trust & Risk Intelligence

OpenHuman includes an intelligence layer designed to help you reason about credibility, information quality, and potential risks across your connected sources.

**Scam and impersonation signals.** Behavioral patterns associated with scams, impersonation, or coordinated abuse can surface as warnings. Signals come from patterns, not from sharing individual message content.

**Contextual dynamic trust.** Trust is contextual, credibility in one domain does not automatically transfer to another. OpenHuman represents trust through aggregated artifacts and historical accuracy rather than static scores.

**Advisory, not enforcement.** Trust and risk outputs are advisory signals to inform your judgment. OpenHuman does not ban users, remove messages, or enforce moderation decisions.

***

## Shared environments

In team or community settings, privacy remains user-centric. Each user's connected sources are scoped to their account; admins do not get a backdoor into other users' Memory Trees.

Community-level intelligence is derived from aggregated and anonymized signals, never from direct access to individual message content.
