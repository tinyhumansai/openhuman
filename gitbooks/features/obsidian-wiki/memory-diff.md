---
description: >-
  Git-backed change tracking for memory. Every sync is committed to a snapshot
  ledger, so the agent can ask "what changed since I last looked?"
icon: git-compare
---

# Memory Diff

The [Memory Tree](memory-tree.md) tells the agent what it knows. **Memory Diff** tells it what _changed_. It is a derived ledger that records the state of every memory source over time, so any agent (or you) can ask: what's new, what was edited, what disappeared - since the last sync, since I last read it, or since a named baseline.

The chunk store (`mem_tree_chunks`) stays authoritative. The diff ledger is a read-only view built _from_ already-ingested data - so snapshots cost zero API calls. Source: `src/openhuman/memory_diff/`.

***

## It's a git repository

The whole thing is a real [libgit2](https://libgit2.org/) repository living at `<workspace>/memory_diff/repo` (`git_store.rs`). Rather than invent a snapshot format, OpenHuman maps memory-change tracking straight onto git's native primitives:

| Memory-diff concept | Git primitive                                       |
| ------------------- | --------------------------------------------------- |
| Snapshot            | Commit (the snapshot id **is** the commit SHA)      |
| Item                | One flat blob, named by the (encoded) item id       |
| Source              | A subtree under `<source_id>/` in the root tree     |
| Checkpoint          | Annotated tag `ckpt_<uuid>` at HEAD                 |
| Read marker         | Ref `refs/openhuman/read/<source_id>` → commit SHA  |
| Diff                | A git tree-to-tree diff scoped to one source's path |

Snapshot metadata that has no natural git home - source kind, label, trigger (`auto` / `manual`), item count, millisecond timestamp - rides along in the **commit message as trailers** (`Source-Id:`, `Trigger:`, `Item-Count:`, `Taken-At-Ms:`, …) and is parsed back out on read.

***

## The snapshot model

After each successful sync, `auto_snapshot_after_sync()` reads the current chunks for that one source out of `mem_tree_chunks`, groups them into one blob per item, and commits them under `<source_id>/`. Crucially, **every other source is carried forward** from the parent commit - so each commit's tree reflects the whole world, even though only one source actually changed.

```
mem_tree_chunks  (authoritative)
        |
        |  sync finishes for source A
        v
  take_snapshot(A)         items grouped, one blob per item
        |
        v
┌──────────────────────────────────────────────┐
│  commit_snapshot                              │
│                                               │
│   root tree = parent tree                     │
│      ├── src_A/   ← rebuilt from new items     │
│      ├── src_B/   ← carried forward unchanged  │
│      └── src_C/   ← carried forward unchanged  │
│                                               │
│   message trailers: Source-Id, Trigger, …     │
└───────────────────────┬───────────────────────┘
                        v
        HEAD ─► commit (= snapshot id / SHA)
```

A diff for source A is then just `git diff <from-tree>..<to-tree>` with the pathspec pinned to `src_A/`. Added / Removed / Modified fall straight out of git's delta status; **Unchanged** is computed as `to_item_count - added - modified`. Item identity is the blob name, so editing an item's content keeps the name (→ `Modified`) while changing its id is `Removed` + `Added`.

All writes serialise through a process-global lock, because git's HEAD/parent bookkeeping is read-modify-write and concurrent commits could otherwise fork history.

***

## What the agent uses it for

The headline use case is **"what changed since I last looked?"** During a conversation the agent calls the `memory_diff` tool (`tools.rs`). Its parameters:

| Param                | Effect                                                                                          |
| -------------------- | ----------------------------------------------------------------------------------------------- |
| _(none)_             | Lists enabled sources with their snapshot counts.                                               |
| `source_id`          | Diffs one source.                                                                               |
| `checkpoint_id`      | Cross-source diff: everything that changed since that named checkpoint.                          |
| `since_read`         | When diffing a source, show changes since the **read marker** rather than since the last sync. Default `true`. |
| `commit`             | When using `since_read`, advance the read marker to head after reading. Default `true`; set `false` to preview without acknowledging. |
| `include_text_diff`  | Include line-level unified diffs for modified items (truncated to ~2000 chars). Default `false`. |

The read-marker mechanic is the turn-to-turn primitive (`diff_since_read` in `ops.rs`): the first call returns the full delta and moves `refs/openhuman/read/<source_id>` up to the current head; the next call returns _only_ what arrived since. So an agent that polls a source repeatedly never re-reads the same news twice. The tool is `ReadOnly` with respect to your data - the only write it performs is advancing that internal marker, never anything in `action_dir`.

Output is concise markdown, e.g.:

```
## Memory Changes (Inbox)

**2 added, 1 modified, 0 removed** (47 unchanged)

### Added
- Invoice #4021 from Acme
- Re: Q3 planning

### Modified
- Standup notes
```

***

## Checkpoints and cross-source diffs

A **checkpoint** is a named baseline across _all_ enabled sources - an annotated git tag at HEAD that records the latest snapshot id per source (`create_checkpoint`). It will even take a fresh snapshot for any source lacking one, so the baseline is complete. Later, `diff_since_checkpoint` walks each recorded snapshot to its current head and aggregates per-source changes into one `CrossSourceDiff` - "everything that's happened across my whole memory since this morning's baseline."

Checkpoints are cheap to prune: `cleanup` deletes tags older than N days, but **snapshot commits are never deleted** - git history _is_ the ledger, and git's delta compression keeps it compact.

***

## Why this matters to you

Because the ledger is real git history, Memory Diff gives the agent's knowledge an **audit trail**:

* **Traceability.** Every change to what the agent knows is a commit with a timestamp, a trigger (`auto` vs `manual`), and an item count.
* **No surprises.** The agent acts on _deltas_, not the whole world each turn - so a single new email gets noticed without re-reading your entire inbox.
* **Recoverable history.** Snapshots are kept indefinitely; you can always reconstruct what a source looked like at any past point.
* **Cheap.** It's built from data already ingested by the Memory Tree, so tracking change costs no extra model or API calls.

***

## See also

* [Memory Tree](memory-tree.md) - the authoritative knowledge base that snapshots are derived from.
* [Auto-fetch from Integrations](auto-fetch.md) - what triggers the syncs that produce new snapshots.
* [Obsidian Wiki](README.md) - the Markdown vault these sources ingest into.
* [Subconscious Loop](../subconscious.md) - the background loop that reviews new memory changes for actionable items.
