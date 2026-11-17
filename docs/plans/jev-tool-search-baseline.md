# `tool_search` ranking: baseline and results

Measured 2026-09-22 with `cargo run -p openhuman-cli --bin tool-search-bench`,
live Jev (`jev-1.13` through the TinyHumans System One proxy, signed-in
session) and the managed `embedding-v1` embedder. Catalogue: every tool the
orchestrator session registers (215) plus the recorded Composio catalogues
under `tests/fixtures/composio_*.json` (1,000 actions across gmail, slack,
github, notion, googledrive, googlesheets, reddit, facebook, instagram) as the
deferred per-action tools a signed-in workspace synthesises. Intents:
`tests/fixtures/tool_search/intents.jsonl` — 160 hand-written requests, 66
labelled with a Composio action, 63 with a core tool, 31 that no tool should
answer.

Before this work the orchestrator reached a Composio action only through
`delegate_to_integrations_agent` → an `integrations_agent` sub-run whose
toolkit was narrowed by `rank_tools_by_prompt` (the `overlap` row). That
delegate has since been removed from the orchestrator: search-then-call is
its only route to an integration action. Every
`tool_search` row is one search followed by a direct call of the tool it
returns; no sub-agent.

## Rankers

| ranker | rows | top-1 | top-3 | retriever recall@20 | needless (of 31) | errors | p50 ms | p95 ms |
|---|---|---|---|---|---|---|---|---|
| bm25 | 160 | 22.5% | 38.0% | 70.5% | 26 | 0 | 28 | 29 |
| overlap (`rank_tools_by_prompt`, the sub-agent's narrowing today) | 160 | 35.7% | 52.7% | 69.0% | 27 | 0 | 25 | 27 |
| Jev, BM25 top-20 then decide | 160 | 57.4% | 62.0% | 70.5% | 1 | 21† | 1542 | 3598 |
| Jev, embedding top-20 then decide (**product default**) | 160 | 62.0% | 66.7% | 86.8% | 1 | 5 | 1527 | 2611 |
| Jev only, family then decide (BM25 cut for >254) | 160 | 62.0–64.3% | 67.4–69.0% | 70.5% | 1 | 0–2 | 1275 | 2138 |
| Jev, family then decide, embedding cut for >254 | 160 | 62.8% | 67.4% | 86.8% | 1 | 4 | 1287 | 2018 |

† the 3 s per-evaluation deadline of an earlier build; raised to 6 s in the
product and 20 s in the bench, after which errors are the residual proxy
timeouts shown on the other rows.

## Composio actions — the heavy catalogue

| ranker | labelled | top-1 | top-3 | retriever recall@20 |
|---|---|---|---|---|
| bm25 | 66 | 18.2% | 36.4% | 72.7% |
| overlap | 66 | 34.8% | 50.0% | 68.2% |
| Jev, BM25 top-20 | 66 | 66.7% | 72.7% | 72.7% |
| Jev, embedding top-20 | 66 | 74.2% | 78.8% | 90.9% |
| Jev only, family then decide | 66 | 77.3–83.3% | 86.4–90.9% | 72.7% |
| Jev, family then decide + embedding cut | 66 | 80.3% | 87.9% | 90.9% |

Ranges are two runs of the same configuration: Jev's answers vary by a few
points run to run.

What the rows say:

- **Retrieval was the ceiling.** With BM25 shortlisting, Jev's Composio top-3
  (72.7%) equals BM25's recall@20 (72.7%): Jev picked correctly from
  everything it was shown. A paraphrase ("ping alex" → `SLACK_SEND_MESSAGE`)
  never reached it.
- **Letting Jev pick the family first removes the shortlist for every toolkit
  that fits one choice** (all but GitHub's 500 actions), and Composio top-3
  goes to 86–91%. The remaining misses are near-synonyms
  (`NOTION_APPEND_TEXT_BLOCKS` for `NOTION_ADD_PAGE_CONTENT`,
  `INSTAGRAM_GET_IG_MEDIA_COMMENTS` for `INSTAGRAM_GET_POST_COMMENTS`) and
  GitHub actions the BM25 cut dropped.
- **Embeddings lift recall@20 to 90.9%** on Composio, and that retriever
  with one Jev decision is the product default: `RetrieveThenDecide` over
  `EmbeddingToolRanker`, one proxy round trip. Family-then-decide scores a
  few points higher on Composio at a second round trip and stays available
  through `JevRankerConfig::with_strategy`. Catalogue embeddings are computed
  once per process (19 batches of 64 for this catalogue) and cached through
  the configured storage backend, keyed by the provider's signature; every
  later search embeds only the intent.
- **No embedder, no Jev search.** When the configured embedding provider is
  `none`, `TinyHumansJevRanker` returns an error and the harness answers with
  its own BM25 catalogue — a Jev decision over a lexical shortlist would only
  add a round trip to the same recall.
- **Needless calls collapse**: 26/31 tool-less requests got a BM25 hit; every
  Jev configuration answers at most one, because Jev's `none` option and
  `needs_tool` abstain.
- **Core tools score lower under Jev than Composio** (46–54% top-3) because
  `tinytools-jev` now abstains when `needs_tool < 0.5` or `none` beats the
  best option, and many core-tool intents ("what did I tell you about my
  dog?", "show me my todos") read as answerable without a tool. In the
  product these tools are `Direct` — on the wire, never searched — so the
  Composio column is the one `tool_search` is measured by.
- **Latency** is 1.3 s p50 for the family strategy (two proxy round trips,
  the second stage's families evaluated concurrently), against a sub-agent
  run of several model calls.

## Reproducing

`tool-search-bench` now lives in the `profile/` crate of [openhuman-benchmarks](https://github.com/tinyhumansai/openhuman-benchmarks); run these from a checkout of that repository.

```text
cargo run --manifest-path profile/Cargo.toml --bin tool-search-bench -- --ranker all --misses
cargo run --manifest-path profile/Cargo.toml --bin tool-search-bench -- --ranker jev --family --embedding
cargo run --manifest-path profile/Cargo.toml --bin tool-search-bench -- --dump-catalogue
```

`OPENHUMAN_BACKEND_API_KEY` (or `TYPESAFE_API_KEY`) selects the key; without
one the bench ranks through the signed-in TinyHumans session exactly as the
product does.
