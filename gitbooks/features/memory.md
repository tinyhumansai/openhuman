---
description: >-
  Memory v2: a pluggable engine that stores your documents, conversations and
  learnings, answers questions with citations, and keeps a brief for new chats.
icon: brain
---

# Memory

OpenHuman's memory lets the agent remember you across chats: what you are working on, what you prefer, what is in your documents. Memory is a feature of the app, not a hidden database. You pick who stores it, see what is stored, and delete anything.

Open it from **Connections → Memory**. It has six tabs (chips): **Engine**, **Ask**, **Learnings**, **Conversations**, **Documents** and **Context**. The old `/brain` and `/settings/memory-engine` addresses redirect there.

## Engines

An engine stores your memory and answers questions about it. There are two:

| Engine | What it is | Needs |
| --- | --- | --- |
| **TinyHumans** | Hosted CortexDB run by TinyHumans | You are signed in |
| **CortexDB** | Your own CortexDB | An endpoint and an API key (kept in the OS keychain) |

Pick one on the **Engine** tab. If you are signed out and have no CortexDB key, memory is **off**: the agent has no memory tool, nothing is stored, and the Memory page tells you why.

## What the engine does

- **Recall** answers a question in plain language and cites the stored items it relied on. The engine implements it.
- **Fetch** is raw search over stored items with metadata filters (folder, repo, thread, source, time window). Both launch engines support hybrid search only.
- **Store** takes three kinds of items: documents, conversations and learnings.

## What gets stored

### Documents

Add a **source** on the **Documents** tab. A source is one of:

| Kind | Target |
| --- | --- |
| `folder` | A folder on your computer |
| `file` | A single file |
| `link` | A web page |
| `github` | A repository (`owner/repo`) |
| `rss` | A feed URL |
| `composio` | A connected integration (Composio toolkit) |

Sources sync when you press sync and on a schedule you set per source. An unchanged file that syncs again is not stored twice. Removing a source can also forget the items it produced.

### Conversations

After a few committed turns in a thread, or once a thread has been idle for a while, OpenHuman stores the recent turns as one conversation item. Defaults are every 4 turns or 120 seconds idle; change them on the **Conversations** tab, which also lets you turn it off. Tool calls are stored by name and id only, never their arguments. Anything still buffered is flushed when you quit the app, within a short time limit.

### Learnings

A learning is one durable statement: a preference, fact, procedure or correction. The agent saves them with its `memory` tool (`learn` action), and you can add or delete them on the **Learnings** tab.

Everything is scrubbed for secrets and personal identifiers before it leaves your machine.

## The agent's memory tool

The agent has one tool, `memory`, with four actions: `recall`, `fetch`, `learn` and `forget`. OpenHuman's own [MCP server](../developing/mcp-server.md) exposes the same abilities to other apps as `memory.recall`, `memory.fetch`, `memory.list`, `memory.learn` and `memory.forget`.

## context.md

Every six hours (and on demand from the **Context** tab) OpenHuman asks the engine for a short brief about you: who you are, active work, preferences and standing instructions, recent important events, plus your learnings. It is saved as `<workspace>/memory/context.md`, trimmed to a token budget (default 2000), and placed at the start of **new** chats only. A chat you resume keeps the prompt it started with, so it does not see a newer brief. Interval, budget and an on/off switch are on the **Context** tab.

## Importing your previous memory

If OpenHuman finds memory from the earlier (v1) version, the Memory page offers a one-time import. It uploads that data to the engine you selected, so it only starts after you explicitly consent. It resumes if interrupted.

## Removed in v2

The memory tree, graph view, goals list, people and contacts, learning profile, `MEMORY.md` and `PROFILE.md`, the Obsidian vault, the local TinyCortex engine and the Supermemory, Mem0, Cognee and agentMemory engines are gone, as is engine migration. See the [spec](https://github.com/tinyhumansai/openhuman/blob/main/docs/specs/memory-v2.md).

## See also

- [Memory architecture](../developing/architecture/memory.md)
- [Pluggable engines](../developing/engines.md)
- [Privacy and security](privacy-and-security.md)
