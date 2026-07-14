---
description: >-
  Open OpenHuman's memory as an Obsidian vault so you can read, edit, and link
  the agent's notes by hand, and have it pick up your edits.
icon: book-open
---

# Connect OpenHuman to Obsidian

**Goal:** browse and edit the assistant's memory as plain Markdown in [Obsidian](https://obsidian.md), and have your edits flow back into what the agent knows.

OpenHuman's memory isn't a black box: the same chunks the agent reasons over are written as `.md` files in an Obsidian-compatible vault inside your workspace. See [Memory](../features/obsidian-wiki/) for the full picture; this guide just gets you connected.

***

## Prerequisites

* [Obsidian](https://obsidian.md) installed (free).
* OpenHuman set up with at least one source connected, so there's memory to look at. See [Create my personal AI assistant](personal-assistant.md).

## Privacy implications

* The vault is **local**. It lives in your workspace folder (`…/wiki/`) on your machine. Opening it in Obsidian doesn't upload anything.
* Obsidian reads local files directly; no OpenHuman account or backend is involved in browsing the vault.
* Anything you type into the vault becomes part of what the agent can read on its next ingest. Treat it like writing into the assistant's memory.

***

## Steps

### 1. Open the vault from OpenHuman

Go to the **Memory** tab and click **View vault in Obsidian**. This opens your `…/wiki/` folder as a vault in Obsidian.

If you'd rather open it manually, point Obsidian at the `wiki/` folder inside your data folder:

| Platform | Vault path |
| -------- | ---------- |
| macOS / Linux | `~/.openhuman/…/wiki/` |
| Windows | `%USERPROFILE%\.openhuman\…\wiki\` |

### 2. Explore the structure

Inside the vault you'll find auto-generated summaries organized by source, topic, and date, plus a place for your own free-form notes. Each summary file carries provenance in its frontmatter so you can trace a claim back to its source. See [Memory Tree](../features/obsidian-wiki/memory-tree.md).

### 3. Add your own notes and links

* Drop in your own Markdown notes.
* Build `[[wiki-links]]` between notes by hand.
* Correct or annotate anything the agent summarized.

### 4. Let the agent pick up your edits

Your changes are read on the next ingest, so you don't have to import anything. The agent will see notes you added and links you drew.

***

## Success checks

* [ ] Obsidian opens the vault and shows OpenHuman's summary files.
* [ ] You can open a summary and see source/time provenance in its frontmatter.
* [ ] A note you add by hand is still there after OpenHuman runs (it isn't overwritten), and the agent can reference it in a later chat.

## Common failures

| Symptom | Cause | Fix |
| ------- | ----- | --- |
| Vault opens but is nearly empty | No sources ingested yet | Connect a source and wait an auto-fetch cycle (~20 min); see [personal assistant guide](personal-assistant.md) |
| "View vault in Obsidian" does nothing | Obsidian isn't installed, or the OS couldn't hand off the folder | Install Obsidian, then open the `wiki/` folder manually (paths above) |
| Your edits seem ignored | The next ingest hasn't run, or you edited inside an auto-managed summary block | Give it a cycle; put your own content in your own notes rather than inside generated blocks |

## Recovery

* The vault is just files. If Obsidian shows something odd, close it and reopen the folder. You can't break OpenHuman by browsing.
* If you deleted a note you wanted, the memory database still holds the underlying chunk; the summary can regenerate.

## See also

* [Memory](../features/obsidian-wiki/): how the vault is produced and organized.
* [Keep sensitive data private](privacy-sensitive-data.md): why the vault being local is the whole point.
