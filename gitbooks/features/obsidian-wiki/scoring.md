---
description: >-
  How OpenHuman scores every chunk before it enters the Memory Tree - weighted
  signals gate admission, entity extraction enriches it, and an inverted index
  plus vector embeddings make it recallable.
icon: scale
---

# Memory Scoring & Ranking

Not every chunk deserves a place in the Memory Tree. A "thanks!" reply, an email footer, or a calendar auto-notification carry almost no signal, and folding them into summary trees only dilutes the result and burns LLM tokens. Scoring is the gate: a per-chunk pass that runs **after chunking and before the chunk is appended to the L0 buffer**, deciding whether the chunk is worth keeping, enriching it with extracted entities, and indexing it for retrieval.

The entry point is `score_chunk` in [`src/openhuman/memory_tree/score/mod.rs`](../../../src/openhuman/memory_tree/score/mod.rs). It is a pure function - it computes a result but does not touch the store; callers persist based on `ScoreResult::kept`.

***

## Why scoring exists

Two goals, both in service of a dense, relevant tree:

* **Keep the tree signal-rich.** Trees compress _and_ navigate. Admitting noise makes both worse - summaries get vaguer and retrieval surfaces junk.
* **Control cost.** The deep path can call an LLM for entity extraction and importance rating. Scoring is structured so that obvious keeps and obvious drops never pay that cost - only genuinely borderline chunks consult the model.

***

## The signals

`score_chunk` computes a bag of independent signals, each normalised to `[0.0, 1.0]`, defined in [`score/signals/`](../../../src/openhuman/memory_tree/score/signals/). They are combined into a single weighted total and stored alongside it in `mem_tree_score` so every admit/drop decision stays auditable.

| Signal            | What it measures                                                                                  | Default weight |
| ----------------- | ------------------------------------------------------------------------------------------------- | -------------- |
| `token_count`     | Plateau over chunk size: 0 below `TOKEN_MIN` (10), ramps to 1 by `TOKEN_RAMP_LOW` (30), eases back to 0.5 toward `TOKEN_MAX` (8000). | 1.0 |
| `unique_words`    | Type-token ratio noise detector; low lexical diversity scores low, very short messages return a neutral 0.5. | 1.0 |
| `metadata_weight` | Base weight per `SourceKind` (Email > Document > Chat).                                            | 1.5 |
| `source_weight`   | Per-`DataSource` weight inferred from `provider:<name>` tags, with `SourceKind` defaults.          | 1.5 |
| `interaction`     | Engagement-tag bonus (`sent`, `reply`, `dm`, `mention`); absent tags return 0.5 so silent content isn't penalised. | 3.0 |
| `entity_density`  | Distinct entities per token, capped at ~1 entity / 100 tokens. More entities → more substantive.  | 1.0 |
| `llm_importance`  | LLM-derived importance rating in `[0.0, 1.0]`. Off by default; weight `2.0` once an LLM extractor is wired in. | 0.0 |

`interaction` is deliberately the strongest signal - direct user engagement is the clearest proxy for "this mattered to a human." Weights live in `SignalWeights` ([`signals/types.rs`](../../../src/openhuman/memory_tree/score/signals/types.rs)); `combine` / `combine_cheap_only` in [`signals/ops.rs`](../../../src/openhuman/memory_tree/score/signals/ops.rs) produce the normalised total (the cheap variant excludes the `llm_importance` term).

***

## The admission gate

```
chunk --> regex extract --> cheap signals --> combine_cheap_only
                                                     |
              +--------------------------------------+--------------------------------------+
              |                                      |                                      |
     total >= DEFINITE_KEEP (0.85)         DEFINITE_DROP < total < KEEP            total <= DEFINITE_DROP (0.15)
        admit, skip LLM                  borderline -> LLM extract,                   drop, skip LLM
                                          merge, recompute, recombine
              |                                      |                                      |
              +--------------------------------------+--------------------------------------+
                                                     v
                              final total >= DROP_THRESHOLD (0.3) ?  --> admit / drop
                                                     v
                              extract entities --> canonicalise --> index + embed
```

The three band constants are defined in `mod.rs`:

* `DEFAULT_DEFINITE_KEEP = 0.85` - cheap total at or above this is admitted without the LLM.
* `DEFAULT_DEFINITE_DROP = 0.15` - cheap total at or below this is dropped without the LLM.
* `DEFAULT_DROP_THRESHOLD = 0.3` - the final admission cutoff applied after any LLM augmentation.

Only chunks whose cheap total lands **strictly between** the two definite bands pay for an LLM call - that is where the importance signal is most informative. A short-circuit on either side skips it.

Two refinements sit on top. Chunks tagged `priority_high` at ingest (GitHub commit messages, closed/merged issues and PRs) get a `PRIORITY_BOOST` of `+0.25` (clamped to 1.0) so high-signal source material clears the gate and ranks higher. And a guard drops "tiny, entity-free" chatter - content under `TOKEN_MIN` tokens with no extracted entities - so it can't squeak through on metadata priors alone (priority-tagged chunks bypass this guard).

Dropped chunks still get a score row written for diagnostics, with a `drop_reason`; their chunk row survives for provenance, but no buffer or summary references them.

***

## Entity extraction

Extraction enriches a chunk and feeds both the `entity_density` / `llm_importance` signals and the index. It is pluggable via the `EntityExtractor` trait in [`score/extract/`](../../../src/openhuman/memory_tree/score/extract/), and runs in two stages:

* **`RegexEntityExtractor`** - always on, deterministic, cheap. Once-compiled patterns pull mechanical identifiers: email, URL, handle (`@alice` and Discord-style `alice#1234`), and hashtag. UTF-8 safe (spans are char offsets).
* **`LlmEntityExtractor`** - consulted only on borderline chunks. A single structured-JSON call asks the model for semantic NER (Person / Organization / Location / Topic / …) plus an importance rating, with span recovery and a soft warn-and-empty fallback on transport failure.

The two are chained by **`CompositeExtractor`**, which runs a sequence of extractors and tolerates per-extractor failures. Outputs are merged (`ExtractedEntities::merge` deduplicates entities and takes the max importance), then **canonicalised** by [`resolver.rs`](../../../src/openhuman/memory_tree/score/resolver.rs) - lowercasing emails, stripping leading `@`/`#`, and assigning stable `canonical_id` strings - so the same person or topic resolves to one identity across chunks.

***

## The entity index & graph

Canonical entities for each kept chunk are written to **`mem_tree_entity_index`**, an inverted index mapping `entity_id → node_id` ([`store.rs`](../../../src/openhuman/memory_tree/score/store.rs)). This is the connective tissue the rest of the Memory Tree reads from:

* **Retrieval** resolves a query's entities against the index to find candidate nodes.
* **Topic routing** uses entity hotness to decide which entities deserve their own topic tree.
* **The Memory graph** (the force-directed visualization in the Intelligence tab) is drawn from co-occurrence edges - two entities mentioned in the same chunk get an undirected edge (`graph::pairs_from_entities`), written in the same transaction as the index so the two never diverge.

***

## Embeddings for semantic recall

Scoring also produces vectors. The embedder in [`score/embed/`](../../../src/openhuman/memory_tree/score/embed/) turns each chunk (and later, summary) into a fixed `EMBEDDING_DIM = 1024`-float `Vec<f32>`, packed into a SQLite BLOB, so retrieval can rerank candidates by cosine similarity rather than relying on the entity index alone.

The active embedder is selected by `build_embedder_from_config` ([`embed/factory.rs`](../../../src/openhuman/memory_tree/score/embed/factory.rs)) walking a resolution ladder, identical for read and write paths:

1. **Explicit Ollama override** (`memory_tree.embedding_endpoint` + `embedding_model`) - power users / E2E rigs.
2. **Local Ollama** via the unified `embeddings` workload setting - the "Memory embeddings" checkbox in [Local AI](../model-routing/local-ai.md) Settings.
3. **User-configured OpenAI-compatible** endpoint (`OpenAiCompatEmbedder`, e.g. LM Studio).
4. **Managed cloud** (`CloudEmbedder`, OpenHuman backend / Voyage) - the default once logged in.
5. **No provider** - the read path falls back to `InertEmbedder` (zero vectors) so retrieval still runs; the write path returns `None`, skips embedding, and flags `semantic_recall` degraded so the chunk can be re-embedded later.

Embeddings run on the background workers, not the ingest hot path, so a burst of new sources never blocks the UI. Trees give compression and navigation; embeddings keep similarity search working underneath them.

***

## See also

* [Memory Trees](memory-tree.md) - the pipeline scoring sits inside.
* [Retrieval](retrieval.md) - how the index and embeddings are queried.
* [Obsidian Wiki](README.md) - the Markdown vault scored chunks land in.
* [Token Compression](../token-compression.md) - why keeping the tree dense matters.
