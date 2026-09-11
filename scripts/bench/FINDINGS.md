# Agent-scale benchmark — findings

Measured with `scripts/bench/run-agent-scale.sh` against a release
`openhuman-core` on one Linux box, mocked LLM, concurrency 8, `fresh` thread
mode. Every number below is reproducible with the commands shown.

## Summary

Under sustained agentic load the core degrades: per-turn latency climbs
linearly, throughput falls to about a third of its starting rate, and RSS grows
without plateauing.

**It is not a memory leak, and it is not a missing index.** Memory recall loads
the *entire* memory namespace — every document and every vector chunk — on every
turn, decodes each embedding, and scores them in-process behind a single
connection mutex. Cost is Θ(memories stored) per turn, so a session's total work
is quadratic and the throughput ceiling falls as memories accumulate. RSS grows
because the working set is the stored data.

## Evidence

### 1. It tracks stored data, not process uptime

A fresh core process pointed at an already-populated workspace **inherits the
full cost immediately** rather than starting fast:

| Run | Workspace | Start latency | End latency |
| --- | --- | --- | --- |
| C1 | empty | 111 ms | 438 ms |
| C2 | C1's, **fresh process** | **532 ms** | 534 ms |

```bash
scripts/bench/run-agent-scale.sh --duration-ms 240000 --keep-workspace --out-dir target/bench/C1
scripts/bench/run-agent-scale.sh --duration-ms 120000 --workspace target/bench/C1/workspace --out-dir target/bench/C2
```

A leak would have been left behind with the old process. This rules that out.

### 2. Turning the memory subsystem off removes every symptom

```bash
scripts/bench/run-agent-scale.sh --duration-ms 240000 --tool-depth 0            # memory on
scripts/bench/run-agent-scale.sh --duration-ms 240000 --tool-depth 0 --memory-off
```

| | memory ON | memory OFF |
| --- | --- | --- |
| latency over 4 min | 111 → 438 ms | **flat ~79 ms** |
| throughput retained | 33% | **102%** |
| RSS growth | +109 KiB/turn | **−0.65 KiB/turn** |
| CPU per turn | 253 ms | **28 ms** |
| turns completed | ~8,600 | **~24,500** |

Nothing else in the turn path shows the behaviour. Tool depth is irrelevant —
`--tool-depth 0` (no `memory_search` calls at all) degrades identically, so this
is the implicit per-turn recall, not the agent's memory tool.

### 3. What the recall actually does

At 8,600 turns the store held 9,660 documents and 10,127 chunks, all in one
`global` namespace. Per recall it loads all of both:

```text
chunks : 10127 rows, 39.6 MiB of embeddings materialized
docs   : 9660 rows
        ~61 ms of SQL per recall, warm cache
```

The indexes exist and are used (`idx_vector_chunks_ns_doc`,
`idx_memory_docs_ns_updated` — `SEARCH … USING INDEX`). They cannot help: the
query has no selective predicate, it wants every row in the namespace.
`query_namespace_hits_excluding_session` does take a `limit`, but applies it
after loading and scoring everything.

### 4. It is the read path, not write contention

Reads and writes share one connection, and `--memory-off` disables both — so on
its own it cannot say which is expensive. Against an already-populated
workspace, with writes off but recall still scanning every turn:

```bash
scripts/bench/run-agent-scale.sh --duration-ms 90000 --tool-depth 0 \
  --memory-writes-off --workspace <populated>
```

| | throughput | p50 |
| --- | --- | --- |
| reads + writes | 14.6/s | 540 ms |
| **reads only** | **15.8/s** | **499 ms** |

Removing every write buys ~8%. The recall scan is the cost; write contention is
a rounding error.

### 5. The single connection mutex sets the ceiling

`UnifiedMemory` owns one `Mutex<Connection>`, so the scan serializes across
concurrent turns. Throughput saturates accordingly:

| concurrency | p50 | throughput |
| --- | --- | --- |
| 1 | 212 ms | 4.7/s |
| 2 | 239 ms | 8.2/s |
| 8 | 550 ms | 14.5/s |

Saturation at ~14.5/s implies ~65 ms per turn in a serialized section, matching
the measured ~61 ms scan. Adding cores cannot raise this, and it falls as the
namespace grows.

## The cost is work volume, not synchronization

Two plausible optimizations were implemented and measured. Neither helped. Both
were aimed at how the scan is *synchronized*; the thing that actually costs is
how much the scan *touches*.

The measurement that settles it, at 5,162 chunks, concurrency 8:

```text
p50 = 326 ms, throughput 24.1/s
Little's law: 24.1 × 0.326 = 7.85 ≈ concurrency 8   → every worker busy all turn
CPU: 6.85 of 14 cores (49%),  295 ms of CPU per turn
```

295 ms of **CPU** per turn, against 28 ms with memory off. That is not a queue
behind a lock — it is real work being done, at half the machine's capacity, with
no contention to remove. Any fix that leaves the same rows being read, decoded
and scored will move this number by a few percent at best, which is exactly what
both attempts did.

### Two fixes that failed — do not repeat them

**Attempt 1 — decode embeddings outside the connection lock.** Rationale: ~10k
allocations and ~10M little-endian conversions per recall were happening inside
the critical section. Interleaved A/B, 2 reps, identical populated workspaces:

| arm | rep 1 | rep 2 | mean |
| --- | --- | --- | --- |
| baseline | 14.87/s | 14.37/s | 14.62/s |
| decode outside lock | 14.42/s | 14.18/s | 14.30/s |

**Attempt 2 — a read-only connection pool for the two O(N) scans.** Rationale:
WAL supports concurrent readers, but one mutex-guarded connection serializes
them, and half the cores were idle. Verified genuinely in use (the pooled arm
held ~19 more file descriptors, and a unit test asserted connections were
actually parked rather than silently falling back). Measured at two corpus sizes:

| corpus | arm | rep 1 | rep 2 | mean |
| --- | --- | --- | --- | --- |
| 2,121 chunks | baseline | 37.21/s | 35.64/s | 36.43/s |
| 2,121 chunks | read pool | 36.04/s | 34.56/s | 35.30/s |
| 5,162 chunks | baseline | 24.07/s | 23.91/s | 23.99/s |
| 5,162 chunks | read pool | 23.81/s | 23.55/s | 23.68/s |

Both point estimates are slightly negative, consistently, across four pairs.
Both changes were reverted; `vendor/tinymemory` is byte-identical to upstream
`f8bd9af` (`git diff f8bd9af` is empty, 855 tests pass).

**The lesson, stated plainly so it is not re-learned:** the recall path is not
lock-bound and not I/O-bound. It is doing ~253 ms of CPU per turn touching every
row in the namespace, twice over. Only reducing what is touched — items 1, 2, 3
and 7 below — can change that. Micro-optimizing around the scan has now been
measured twice and found worthless.

## Deeper dive — where the cost actually is

The first pass framed this as "recall is O(N), needs a vector index". That is
true but it is the *last* thing to fix, not the first. Looking at the caller
side changes the picture: most of the work is not needed at all.

### The turn does more recalls than you would guess

**4.26 embedding calls per turn**, measured identically across three independent
runs (4.26 / 4.27 / 4.26 — `mock-stats.json` vs `driver.json`). Each recall
embeds its query, so that count is a direct proxy for recall operations. Per
turn there are **three full-namespace SQL recalls plus one vector-only query**,
all before the LLM call, all blocking:

| recall | query | limit | namespace |
| --- | --- | --- | --- |
| citations | user message | 5 | `global` |
| working memory | `"working.user {msg}"` | 5 | `global` |
| prior conversations | `"conversation_memory {msg}"` | 12 | `conversation_memory` |
| situational prefs | user message | 5 | `user_pref_situational` |

Two of them scan `global`. Sub-agents do **not** re-recall (they inherit the
parent's block), so this is per user turn, not per agent.

### What all that buys

At most **9 lines and ~2000 characters** — roughly 500 tokens — reach the
prompt: three each from working memory, prior conversations and cross-chat, each
hard-capped. Citations do not reach the prompt at all; they populate
`last_turn_citations` for the UI.

Measured waste on the vector side: over 20 queries against a real 2,121-chunk
corpus, **99.73%** of chunks scored fell below the 0.4 relevance floor. About
six chunks per query clear it.

And the cosine is not the expensive part — scoring 2,121 chunks takes **3.5 ms
in JavaScript**, so well under a millisecond in Rust. The cost is re-reading and
re-decoding the corpus from SQLite every turn, plus repeated full-content
normalization on the keyword side.

### The clearest single defect

The working-memory recall builds the query string `"working.user {user_message}"`
— a text hack meant to bias ranking — scans all of `global`, takes the top 5,
and **then** filters `key.starts_with("working.user.")`
(`src/openhuman/memory/agent/memory_loader.rs:218-232` in the pre-change code;
this PR removed the file's loader implementation).

So it scans the entire namespace to find entries identified by a known key
prefix. In the benchmark corpus:

```text
global docs scanned per turn : 2025
docs with key 'working.user.': 0
```

Every one of those scans returned nothing usable. This is not only slow, it
**degrades silently**: as autosaved chat fills `global`, the chance that a
`working.user.*` entry survives into the global top-5 falls toward zero, so the
feature quietly stops working long before anyone profiles it.

> Caveat worth checking with someone who owns the sync path: I found **no
> in-repo writer** of `working.user.*` keys at all — only the query, test
> fixtures, and a doc comment describing them as "sync-derived profile facts".
> If nothing writes them in current builds, this block is permanently empty and
> the scan is pure cost. I could not confirm either way.

### Why `global` grows without bound

Every autosaved user message is stored with an empty namespace
(`src/openhuman/agent/harness/session/turn/core.rs:709`), which
`sanitize_namespace` maps to `global` — the same namespace the two hot recalls
scan. The corpus above is 2,024 `user_msg:*` documents and one other.

Namespace partitioning already exists and is already used for
`conversation_memory` and `user_pref_situational`. It is simply not applied to
the two expensive calls.

### The ceiling is the lock, and there are idle cores

The box has **14 cores**; the memory-on run used **7.2**. So this is not CPU
saturation. `UnifiedMemory` holds a single `Mutex<Connection>` and the recall
path acquires it ~8 times per call, so ~61 ms per turn of serialized SQL caps
throughput near 16/s — matching the measured 14.5/s — with half the machine
idle.

## What can be done, cheapest first

The first three are in **`src/openhuman/`, not the vendored crate**, and reduce
how much is scanned rather than how fast the scan runs.

**1. Scope the working-memory recall to its own namespace.** The entries are
already identified by a key prefix; give them a namespace and query that instead
of filtering `global` top-5 after the fact. Removes one full `global` scan per
turn and *fixes* the silent-degradation bug — a working-memory entry can no
longer be crowded out by unrelated chat. Uses machinery already in use
elsewhere. **Do this one first even if nothing else is done.**

**2. Take citations off the turn's critical path.** They are UI-only and never
enter the prompt, yet a full `global` scan blocks the response on them. Removes
the second `global` scan from the latency path.

**3. Stop autosaving raw chat messages into `global`.** This is what makes N
unbounded in the hottest namespace. Needs a product answer first: is raw
user-message recall still earning its place now that `conversation_memory`
(transcript-derived durable facts) and the cross-chat JSONL scan exist? If it is
redundant, this is a one-line namespace change that bounds the problem
permanently.

**4. Add a timeout to recall.** Every call site already treats recall as
best-effort (`unwrap_or_default()` throughout, failures logged and skipped) —
but there is **no timeout anywhere**, and the turn blocks. A slow recall stalls a
turn indefinitely today. This is a robustness fix worth making regardless of the
performance work.

Then, in the vendored crate, in this order:

**5. Cache normalized document text.** `keyword_score_for_text` re-normalizes
every document's full content on every recall (three allocations and ~three
passes per call, via `normalize_search_text`). The result depends only on the
document, never on the query, so it is recomputed identically every turn.
Semantics-identical.

**6. ~~A read-only connection pool.~~ Tried and measured — it does not help.
See "Two fixes that failed" above.**

**7. Only then, a vector index** (sqlite-vec / HNSW). This is the real answer for
genuinely unbounded semantic search, and the only one that makes recall
sub-linear — but items 1–3 cut N by far more than an index would cut the
constant, and they carry much less risk. There is precedent for bounded
retrieval everywhere else in the crate: `episodic_search`, `event_search_fts`,
segments and entities are all `ORDER BY … LIMIT`. This one path is the outlier.

## Outcome — what shipped and what it bought

Items 1, 3 and 2 were implemented (branch `memory-recall-diet`):

1. **`load_context()` removed.** The per-turn `[User working memory]` /
   `[Prior conversations]` / `[Cross-chat context]` block is gone, taking the
   `MemoryLoader` trait and `DefaultMemoryLoader` with it. Memory tools are
   untouched, so the model still fetches memory on demand.
3. **Autosave moved out of `global`** into `conversation_raw`
   (`CONVERSATION_RAW_NAMESPACE`), applied to the agent turn and the channels
   dispatcher.
2. **Citations overlapped rather than serialized.** They are UI-only but were a
   full recall blocking every reply before the model call; now spawned and
   joined in `take_last_turn_citations()`.

Measured on **fresh** workspaces, 4 minutes, concurrency 8, tool-depth 0 —
i.e. the growth test, since both arms start empty:

| | baseline | recall diet | Δ |
| --- | --- | --- | --- |
| turns completed | 7,806 | **16,601** | 2.13× |
| throughput | 32.5/s | **69.1/s** | 2.13× |
| p50 latency | 232 ms | **102 ms** | 2.3× lower |
| CPU per turn | 223 ms | **82 ms** | 2.7× lower |
| RSS growth | 195 KiB/turn | **26 KiB/turn** | 7.5× lower |
| latency drift over the run | 125 → 439 ms | **86 → 162 ms** | — |
| throughput-held verdict | **fail** (33%) | **pass** (55%) | — |

Namespace change confirmed in the resulting stores: baseline wrote
`global=8202`; the diet build wrote `conversation_raw=17552, global=1`.

### The residual growth is write-side, not read-side

Latency still drifts in the diet build (86 → 162 ms), so the job is not
finished. Running the same build with memory writes also disabled isolates it
completely:

| | throughput | p50 | latency over 4 min |
| --- | --- | --- | --- |
| baseline | 32.5/s | 232 ms | 125 → 439 ms |
| diet | 69.1/s | 102 ms | 86 → 162 ms |
| diet, no memory writes | **105.5/s** | **68 ms** | **75 → 76 ms (flat)** |

Flat, and throughput held **99%**. So every remaining drift is in the memory
*write* path — upsert, embedding, chunk insert against a growing store — and
none of it is left on the read side. That is a smaller and separate problem;
the candidate flagged during the dive is the conversation-store index
(`vendor/tinycortex/.../conversations/store_index.rs`), which folds
`threads.jsonl` from scratch on nearly every operation.

Assistant summaries now follow user messages into `conversation_raw` and use a
unique key, so concurrent sessions neither overwrite one global document nor
leak raw conversation autosaves into default-namespace recall.

## Tried and rejected

- **Decode embeddings outside the connection lock** — implemented, measured, no
  effect. See "Two fixes that failed".
- **A read-only connection pool** — implemented, measured, no effect. See the
  same section.
- **A bounded/approximate candidate set as the first move** — not tried,
  deliberately: it changes which memories surface, and items 1–3 achieve more
  without that cost.

## Separate finding — journal write amplification

Every agent run writes a ~604 KB journal file to `tinyagents_store/journal/`,
for a single trivial turn. One line accounts for ~424 KB: the model-call event
serializes the full system prompt and the complete tool schemas for ~80 tools.
A 4-minute run left ~5 GB behind; with memory off (so more turns complete) it
reached ~13 GB.

This is O(1) per turn, so it is **not** the cause of the degradation above, and
the benchmark does not fail on it. It is flagged because ~600 KB per turn of
mostly-static text is a real cost for long-lived installs.
