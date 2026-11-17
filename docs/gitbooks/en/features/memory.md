---
description: >-
  Memory: a pluggable engine that stores your documents, conversations and
  learnings, refreshes context in the background, and answers questions
  with citations.
icon: brain
---

# Memory

Memory lets the agent remember you across chats: what you are working on, what you prefer, what is in your documents. It is a feature of the app, not a hidden database. You choose who stores it, see what is stored, and delete anything.

Open it from **Connections > Memory**. It has eight tabs: Engine, Ask, Explorer, Learnings, Conversations, Brain, Background and Settings. The old `/brain` and `/settings/memory-engine` addresses redirect there.

## How the agent uses memory

Memory works around every turn, and the agent does not have to ask for it.

- **Before the model answers**, OpenHuman takes a completed *memory pack* from its short-lived cache, when one is available, and starts logging your message and refreshing memory in the background. The first turn can have no pack. A cached pack can hold learnings, beliefs, documents and earlier conversations from a previous lookup; it is labelled as cached context, so the agent can use explicit recall when it needs a current answer. The pack is added to requests for that turn only and never written into the chat.
- **After the answer**, the reply is logged in the background with a one-line result for each tool it used.
- **When a long chat is compacted**, OpenHuman can add completed cached memory to the summary and starts refreshing recall of the folded-away turns. Memory reads never hold up the summary.
- **In the background**, every few minutes, the engine builds beliefs from what was stored, such as "the user prefers short answers" or "deploys happen on Fridays".

Memory chips on an answer show which stored items the pack cited. To see what a turn would get, open **Ask > Pack preview**.

## Engines

An engine stores your memory and answers questions about it. There are two.

| Engine | What it is | Needs |
| --- | --- | --- |
| TinyHumans | Hosted CortexDB run by TinyHumans, reached through the TinyHumans backend | You are signed in, or the host supplies a TinyHumans API key (headless and library hosts) |
| CortexDB | Your own CortexDB, called directly (managed `api-v1.cortexdb.ai` or self-hosted) | A CortexDB API key (kept in the OS keychain), and an endpoint if it is not the managed one |

With either engine, committed memory items live in CortexDB. OpenHuman also keeps scrubbed pending agent writes in a private local queue until they are delivered, plus job and sync bookkeeping. Cached packs stay in process memory for a short time. See [Where your memory is stored](privacy-and-security.md#where-your-memory-is-stored).

Pick an engine on the **Engine** tab. If you are signed out and have neither a CortexDB key nor a host-supplied TinyHumans API key, memory is off. The agent has no memory tool, nothing is stored, and the Memory page tells you why.

## What the engine does

- **Recall** answers a question in plain language and cites the stored items it used.
- **Fetch** is a raw search over stored items with metadata filters (folder, repo, thread, source, time window). Both engines support hybrid search only.
- **Store** takes three kinds of items: documents, conversations and learnings.

## What gets stored

### The brain: documents

The **Brain** tab holds documents that every agent shares, filed by source type: `pdf`, `markdown`, `notion`, `github`, `web`, and one per other source (`gmail`, `docx` and so on). It shows how many documents each source holds. You can search them, add one (pasted text or a file from your computer), and forget a whole source.

Below that, synced sources keep the brain up to date. A source is one of:

| Kind | Target | Filed under |
| --- | --- | --- |
| `folder` | A folder on your computer | `pdf`, `web` (HTML) or `markdown`, by file type |
| `file` | A single file | Same as a folder |
| `link` | A web page | `web` |
| `github` | A repository (`owner/repo`) | `github` |
| `rss` | A feed URL | `web` |

Sources sync when you press sync and on a schedule you set for each source. An unchanged file that syncs again is not stored twice. Removing a source can also forget the items it produced.

### Conversations

Turn logging runs in the background: your message is queued before the model runs, and the reply after it commits. A slow or unavailable memory engine does not delay the answer. Each agent's turns are kept apart. Tool calls are stored by name and id with a one-line result, never their arguments. You can switch turn logging off on the **Conversations** tab, which also lists each agent's stored turns.

### Learnings

A learning is one lasting statement: a preference, fact, procedure or correction. The agent queues them with its `memory` tool (the `learn` action), and you can add or delete them on the **Learnings** tab. A tool acknowledgement saying "queued; not yet saved to memory" confirms local acceptance, not a completed save by the engine. Beliefs the engine built are listed there too, marked "Built belief".

Everything is scrubbed for secrets and personal identifiers before it leaves your machine.

### Past conversations

Chats from before turn logging are not in memory until you sync them. The **Conversations** tab shows how many chats and turns are still unsynced. **Sync past conversations** uploads them after you confirm, in the same form as new turns and without tool arguments. You can close the page while it runs, and syncing again later sends only what is new.

## Exploring what memory holds

The **Explorer** tab shows everything your memory holds, grouped by one property at a time: type, memory node, source, workspace, folder, file, language, repository, link, thread, agent, tool or tag. Each value shows how many items carry it. Click a value to narrow to those items, then group by another property. The breadcrumb at the top takes you back up. The items at each step are listed below. **Open** shows one in full with all its details and a **Forget** button.

On a very large memory, the counts may cover only the items scanned so far. The tab says so when that happens.

## Each agent's own memory

Memory is organized the way a team works:

- **Shared by every agent:** learnings and the brain.
- **Each agent's own:** its conversations. An agent's pack leads with its own history, and other agents' turns appear only briefly.
- **Teams and tenants:** an agent team gets its own memory below its own root (`team:<id>`), apart from everyone else's.

By default an agent's memory id is its definition id. In `config.toml`, `[memory.agents.<id>]` can give a definition another memory id (`agent_id`), another root (`root`), or turn off its per-turn pack (`recall = false`).

A host that runs OpenHuman agents as part of something larger, such as an AI company or a multi-agent product, can bind each agent to its own memory. Use `[memory] agent_id` and `root` on that agent's config, or `AgentSpec::memory(MemoryBinding::new("employee-7").root("team:acme"))` through the [embedding library](../developing/embedding.md).

## The agent's memory tool

The agent has one tool, `memory`, with four actions: `recall`, `fetch`, `learn` and `forget`. Explicit reads have a deadline and a shared budget for the current run; when memory is too slow, the agent gets an error and continues without retrying it in that run. Agent `learn` and `forget` requests are queued locally and delivered in the background, so their acknowledgements do not confirm a remote save or deletion. OpenHuman's own [MCP server](../developing/mcp-server.md) offers the same abilities to other apps as `memory.recall`, `memory.fetch`, `memory.list`, `memory.learn` and `memory.forget`.

## Settings and background work

The **Settings** tab sets the shape of the pack: on or off, its size in tokens, how many learnings, documents and turns it holds, and how often beliefs are built. The **Background** tab lists queued belief builds and recent runs, with a **Run now** button. With the TinyHumans engine, beliefs are built on the service's own schedule.

## Importing your previous memory

If OpenHuman finds memory from the earlier (v1) version, the Memory page offers a one-time import. It uploads that data to the engine you selected, so it starts only after you consent. It resumes if interrupted. Data v1 pulled in from connected apps (Gmail, Slack, Notion, Linear, GitHub, ClickUp and the like) is not imported: reconnect the app and it syncs again. Your conversations, memory sources, learnings and profile come across.

## What is not here

The current memory has no memory tree, graph view, goals list, people and contacts, learning profile, `MEMORY.md` or `PROFILE.md` files, or Obsidian vault. Other memory engines (Supermemory, Mem0, Cognee, agentMemory and the local TinyCortex engine) are gone, and so is migrating between engines. The per-turn memory pack replaces the old compiled `context.md` brief.

## See also

- [Memory architecture](../developing/architecture/memory.md)
- [Pluggable engines](../developing/engines.md)
- [Privacy and security](privacy-and-security.md)
