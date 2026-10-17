---
description: >-
  Where your memory is stored, what stays on your machine, what the backend
  brokers, and which protections are on by default.
icon: shield
---

# Privacy and security

Your workspace files, settings and audio buffers stay on your machine. Your memory does not. Memory items are stored in CortexDB, either the hosted CortexDB that TinyHumans runs for you or a CortexDB you point OpenHuman at. The OpenHuman backend brokers the rest of what has to leave the device: LLM calls, OAuth tokens, search proxying and hosted memory.

## Privacy by design

**You choose where memory lives.** [Memory](memory.md) is stored by the engine you select: hosted TinyHumans (CortexDB behind the OpenHuman backend, which needs sign-in) or your own CortexDB endpoint. With neither, memory is off and nothing is stored. Secrets and personal identifiers are scrubbed from every item before it is sent. Tool-call arguments are never stored. You can delete any item. See [Where your memory is stored](#where-your-memory-is-stored).

**Integration tokens stay with the backend, not your laptop.** OAuth tokens are never written to your disk in plaintext. The OpenHuman backend brokers each integration request, and the core never talks to a third-party API directly.

**OS-level credential storage.** Sensitive local secrets are rooted in your platform's secure keychain: macOS Keychain, Windows Credential Manager or Linux Secret Service. See [OS keyring and secret storage](os-keyring-and-secret-storage.md).

**No training on your data.** Your conversations, memories and personal information are never used to train AI models or improve systems.

**Optional [local AI](model-routing/local-ai.md).** To keep embeddings and summary-tree building on your machine, run a local runtime such as Ollama, pull the models yourself and add it as a provider. Learning and reflection passes, and chat if you choose, can move on-device the same way. OpenHuman does not install the runtime or download models.

## What stays on your machine

| Item | Where it lives |
| --- | --- |
| Committed memory items | Stored in CortexDB by your selected engine (see below). A CortexDB you host on your own machine keeps them there. Pending agent writes are held locally until delivery. |
| Memory bookkeeping and pending writes | Local: scrubbed agent `learn`/`forget` requests await delivery in a private outbox under `<workspace>/memory/`, alongside job queues and sync, backfill and import progress. Sources are listed in `config.toml`; the CortexDB key is kept in the OS keychain. Automatic context packs stay in process memory briefly. |
| Audio capture buffers | Local. Discarded after speech-to-text. |
| Local model state | Local. |

## Where your memory is stored

Memory items are your brain documents, logged conversation turns, learnings, and the facts and beliefs the engine builds from them. They are stored in CortexDB, never in a database on your machine. Which CortexDB depends on the engine:

| Engine | Path | Who can reach it |
| --- | --- | --- |
| TinyHumans (hosted) | The core sends each request over TLS to the TinyHumans backend's `/memory/*` routes, authenticated with your session or TinyHumans API key. The backend applies a per-user rate limit and billing, then forwards to a CortexDB proxy that places every scope under your own tenant root. | Only your account. The proxy derives the tenant from your credential, not from the request, so one user cannot read or write another's memory. TinyHumans operates the service. |
| CortexDB (direct) | The core calls CortexDB itself (`api-v1.cortexdb.ai`, or a self-hosted endpoint) with your own CortexDB API key, kept in the OS keychain. It does not go through the TinyHumans backend. | Whoever holds that key, and the operator of that endpoint. |

Inside either engine, memory sits below a root derived from the active config identity: `org:<id>` for a TinyHumans account, or `org:local-<digest>` for a local identity. On hosted memory this is the tenant root the backend pins, so paths read `org:<id>/ws:main/…`. That way memory can be listed, confined and erased as one tree. Your agents share learnings and the brain. Each agent's conversations are tagged with that agent.

We claim only TLS in transit and per-user tenant isolation for hosted memory. These docs do not promise end-to-end or client-side encryption. The hosted service can read what it stores, which is how it answers recall.

### Deleting memory

- **One item:** use **Forget** in the Memory page's **Explorer**, on a memory chip under an answer, on the **Learnings** tab, or through the agent's `memory` tool (`forget`).
- **One source:** use **Forget source** on the **Brain** tab, or tick **Also delete memory from this source** when you disconnect an integration.
- **Everything:** call the `openhuman.memory_erase_all` RPC (`{"confirm": true}`). It erases all memory the selected engine holds, for good. On hosted memory it calls the backend's `DELETE /memory`, which erases your whole tenant. On a direct CortexDB it erases your own `org:<id>` tree (and the earlier `user:<id>` tree while it is still read). Memory written there before the per-user layout, which other accounts on a self-hosted CortexDB may share, is left for you to delete through your CortexDB account or endpoint. The **Settings** tab of the Memory page has an **Erase memory** card that calls the same RPC after you confirm.

Signing out does not delete stored memory. Neither does disconnecting an integration, unless you tick that box.

## What the OpenHuman backend handles

| Area | What the backend does |
| --- | --- |
| LLM calls | Proxies them under one subscription, then forwards to the underlying provider (Anthropic, OpenAI, Google and others) per the [model router](model-routing/README.md). |
| Web search proxy | The native [web search tool](native-tools/web-search.md) can use the backend proxy, so you do not carry a search API key. Only Exa and Gemini have a managed route. Brave, Querit, Tavily, Seltz, Parallel, TinyFish and Gemini Deep Research are bring-your-own-key and go directly to that provider. SearXNG goes to the instance you configured. With Tavily selected, the `tavily_extract` tool also sends extraction requests, including the URLs being extracted, directly to Tavily. |
| Integration OAuth and tool proxy | Stores tokens and brokers rate-limited requests for [the connected integrations](integrations/README.md). |
| TTS streaming | Streams hosted [text-to-speech](native-tools/voice.md) audio. Audio is generated and discarded, not kept. |

## Permissions and access control

OpenHuman accesses an integration only after you complete its OAuth flow. Each connection has its own scope, and you can revoke any of them at any time from the **Connections** page.

[Memory sources](memory.md) sync on a schedule while they exist. Integration sync is bound by:

- The OAuth scope you granted that integration.
- A per-provider sync interval (for example, Gmail every 15 minutes by default).
- A daily budget per connection that caps API usage.

If you revoke a connection, the next sync stops. Items already stored in your memory engine remain until you forget them.

## Why memory is scrubbed and scoped

Memory only helps if it is safe to keep. Every item goes through secret and PII scrubbing before it is stored. Conversations record tool-call names and ids but never arguments. The agent only sees what a recall or fetch returns at the moment of a turn. You can inspect and delete items from **Connections → Memory** (see [Deleting memory](#deleting-memory)). Scrubbing and scoped retrieval together form the privacy architecture.

## Security

**Encrypted in transit.** All communication between the app and the OpenHuman backend uses TLS. So do hosted memory requests and direct calls to CortexDB's managed API. A self-hosted CortexDB endpoint is only as protected as the URL you configure.

**Key in keyring, ciphertext on disk.** For local secrets that must be saved in app files, OpenHuman stores encrypted ciphertext on disk and keeps the master decryption key in the OS keyring. See [OS keyring and secret storage](os-keyring-and-secret-storage.md).

**Sandboxed execution.** Shell commands and code the agent runs go through a sandbox backend chosen per session: none, the OS jail (Landlock on Linux, Seatbelt on macOS), or a Docker container with no network, dropped capabilities and a read-only root filesystem by default. Skills are not sandboxed separately. A skill runs as its own agent session under the same execution policy as any other turn.

**Working-folder-scoped tools.** The native [filesystem tools](native-tools/coder.md) act inside the agent's working folder. That confinement is part of the autonomy policy, which is off by default. Until you set `[autonomy] enabled = true`, it is not enforced. What is enforced either way is the hard floor: credential stores and system roots. See [Approval gate](approval-gate.md).

**Short-lived tokens.** Authentication tokens between the app and the backend are time-limited.

## Trust and risk signals

OpenHuman has an intelligence layer that helps you judge credibility, information quality and risk across your connected sources.

- **Scam and impersonation signals.** Patterns associated with scams, impersonation or coordinated abuse can surface as warnings. Signals come from patterns, not from sharing individual message content.
- **Contextual trust.** Trust is contextual. Credibility in one domain does not automatically carry over to another. OpenHuman represents trust through aggregated artifacts and historical accuracy rather than static scores.
- **Advisory only.** Trust and risk outputs inform your judgment. OpenHuman does not ban users, remove messages or enforce moderation decisions.

## Shared environments

In team or community settings, privacy stays user-centric. Each user's connected sources are scoped to their account, and admins get no backdoor into other users' memory. Community-level intelligence comes from aggregated, anonymized signals, never from direct access to individual message content.
