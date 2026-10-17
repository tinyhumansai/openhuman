---
description: >-
  Jev, the small decision model: what it answers, where OpenHuman uses it,
  what it costs and how to configure it.
icon: scale-balanced
---

# Jev

Jev is a decision model, not a text generator. Give it a request and a handful of labelled options, and it returns a calibrated probability for each one in about 150 milliseconds. OpenHuman uses it as a fast, cheap judge wherever a turn must pick from a short list instead of writing prose. That means tool selection during a search and step-by-step browser control.

The models are `jev-1.13` and `jev-latest`, built by TypeSafe AI. OpenHuman reaches them through the TinyHumans System One proxy or, on a bring-your-own-key OpenRouter route, directly. Input costs $0.042 per million tokens and output is free. A typical tool search uses one to two thousand tokens. There is no session, no chain of thought and nothing to stream: one request, one set of probabilities.

## Three question types

Jev answers three kinds of question. They are defined in `tinyjevclient`:

- Choice: probabilities over a set of named options. The caller can take the best one, the top three, or drop everything below a confidence floor.
- Score: one calibrated number for one thing, instead of a choice among several.
- Noul: a yes/no probability. OpenHuman uses it to ask "does this apply at all" instead of "which of these applies".

Tool search sends a Choice and a Noul in the same request. The Choice picks a tool from the shortlist (plus a `none` option), and the Noul says whether the request needs a tool at all. Browser control calls Jev once per step to decide the next action.

## Why retrieval comes first

Jev accepts at most 255 options in one Choice, and its accuracy drops as the list fills with entries unrelated to the request. So OpenHuman never hands Jev a whole tool catalogue. `tinytools-jev` (`vendor/tinyagents/vendor/tinytools/crates/tinytools-jev/`) narrows first and decides second:

1. A retriever (BM25 by default, or an embedding ranker when one is configured) shortlists `retrieval_k` candidates, 20 by default. This step is skipped when the catalogue already fits in one Choice.
2. One request goes to Jev: a Choice over the shortlist plus `none`, and a Noul asking whether the request needs a tool at all.
3. Hits come back ranked by probability. `none` is removed, and anything below `min_probability` (0.05 by default) is dropped.

`JevRankerConfig::with_strategy` picks how the catalogue is narrowed:

- `RetrieveThenDecide` (the default): retrieve the top 20, then decide once. Recall is capped by the retriever, because a paraphrase it misses never reaches Jev.
- `FamilyThenDecide`: one evaluation picks the family first (a toolkit or pack, with anything else in `core`). A second evaluation then runs per chosen family over every member of it. `max_families` defaults to 2 and the evaluations run concurrently. A family that fits in one Choice skips retrieval, so a paraphrase is judged on meaning at both stages instead of being filtered out lexically first.

## How OpenHuman uses it

### Tool search

The tinyagents harness owns the `tool_search` / `tool_call` bridge over every tool registered as deferred. `openhuman-tinyhumans` installs `TinyHumansJevRanker` (`crates/openhuman-tinyhumans/src/jev/ranker.rs`) as the process-wide ranker for that bridge (`crates/openhuman-core/src/agent/tinyagents/discovery/`). The product default uses `RetrieveThenDecide` with the process's embedding provider as the retriever, so candidates are found by meaning, not shared words, before Jev decides.

The ranker resolves its Jev route and credential on every search instead of caching them at install time, because a desktop can sign in and out while the process keeps running. The route is `agent.tool_search.jev_route` (see [Configuration](#configuration)): the TinyHumans proxy, TypeSafe's own API, or OpenRouter's System One API. The built Jev client is cached by route, credential and backend URL, so a stable session does not rebuild an HTTP client on every search.

If the process has no usable embedding provider, `TinyHumansJevRanker` declines and the harness uses its own BM25 ranking. Running Jev over a lexical shortlist would only add a network round trip for no gain.

### Browser control

`crates/openhuman-core/src/modules/browser_task.rs` hands browser tasks to TinyComputer's task members (`StartTask`, then `AwaitTask` until the task pauses or finishes). Tasks stay on the browser surface and the allowed websites.

The decision model is set in `[computer] decision_model`: Jev (through the hosted proxy when signed in, or your OpenRouter key), OpenJev or Sage. A failed step goes to the rescue model (`[computer] rescue_model`, up to `max_rescues` times) before the task fails. An irreversible step pauses as `needs_approval`. The browser tool holds it behind a one-use token, and only `confirm_pending`, through the host approval gate, answers `ContinueTask`.

The TinyComputer module keeps what finished browser tasks learned, per site, through the host's `SiteMemoryHost` interface (`crates/openhuman-core/src/modules/site_memory_host.rs` stores it in `<workspace>/state/computer/sites/`). `StartTask.site` makes the module load that and hand it to the task on the same site:

- The plan a run finished with goes in as the task's flow for the same goal and facts, so no planning is needed.
- The elements it found go in as its grounding hints. A remembered element costs one yes/no instead of a search.

Only a finished run (`done`, or its payment checkpoint) teaches anything. A plan is kept only when its run needed no rescue and holds no fact value its goal does not. A reused plan that fails or needs a rescue is forgotten. Elements holding a fact value or page text are not kept. Entries expire after 30 days unused, and none of it reaches the agent's memory or prompts. `[browser] learn_from_tasks` turns this off, and `modules.browser_forget_sites` forgets one site or all.

## Measured results

These numbers come from [`docs/plans/jev-tool-search-baseline.md`](https://github.com/tinyhumansai/openhuman/blob/main/docs/plans/jev-tool-search-baseline.md). The test used 215 core tools plus 1,000 recorded Composio actions and 160 hand-written requests. Of those, 66 map to a Composio action, 63 to a core tool and 31 should get no tool.

| Ranker | Top-1 | Top-3 | Needless calls (of 31) | p50 latency |
| --- | --- | --- | --- | --- |
| BM25 | 22.5% | 38.0% | 26 | 28 ms |
| Jev, BM25 top-20 then decide | 57.4% | 62.0% | 1 | 1.5 s |
| Jev, embedding top-20 then decide (product default) | 62.0% | 66.7% | 1 | 1.5 s |
| Jev, family then decide | 62.0 to 64.3% | 67.4 to 69.0% | 1 | 1.3 s |

On the Composio-only slice, `FamilyThenDecide` with an embedding cut for oversized families reaches 80.3% top-1 and 87.9% top-3. BM25 reaches 18.2% and 36.4%.

The gap comes mostly from retrieval, not the decision. With `RetrieveThenDecide` and BM25 shortlisting, Jev's Composio top-3 (72.7%) equals BM25's recall@20. Jev chooses correctly from everything it sees, but a paraphrase like "ping alex" for `SLACK_SEND_MESSAGE` never reaches it. Letting Jev pick the family first removes the shortlist for every toolkit that fits in one Choice and lifts Composio top-3 into the high 80s.

Needless tool calls drop sharply under any Jev setup. BM25 answers 26 of 31 tool-less requests with a wrong tool. Jev's `none` option and `needs_tool` Noul let it abstain on all but one.

## Configuration

`ToolSearchConfig` (`crates/openhuman-core/src/config/schema/agent.rs`) is the `[agent.tool_search]` block:

```toml
[agent.tool_search]
ranker = "jev"   # "jev" (default) | "auto" | "bm25" | "compare"
jev_route = "auto"   # "auto" (default) | "tinyhumans" | "typesafe" | "openrouter"
# jev_base_url = "http://127.0.0.1:18080"   # typesafe/openrouter origin override
top_k = 3
```

- `ranker = "jev"`: use the installed decision-model ranker. It falls back to BM25 only when Jev fails or the process has no TinyHumans credential.
- `"auto"`: the same fallback as `"jev"`.
- `"bm25"`: the built-in lexical ranker only, with no network call.
- `"compare"`: serve Jev, and also record the BM25 ranking in the `tool.searched` telemetry. You can compare the two on live traffic without changing what the model sees.
- `jev_route`: where Jev calls go. `"auto"` uses the TinyHumans credential if the process has one, else `TYPESAFE_API_KEY` (direct to TypeSafe), else an OpenRouter key (`OPENROUTER_API_KEY`, or the stored `openrouter` key). `"tinyhumans"`, `"typesafe"` and `"openrouter"` pin one route and never fall through to another credential. Override per launch with `OPENHUMAN_JEV_ROUTE`.
- `jev_base_url`: replaces the API origin of the `typesafe` and `openrouter` routes, for example a metering proxy or a mirror. Remote origins must be HTTPS. HTTP is accepted only for literal loopback IPs. Override per launch with `OPENHUMAN_JEV_BASE_URL`.
- `top_k`: how many matches a search returns when the model does not ask for a number. The default is three, enough to choose from without returning so many schemas that deferring them loses its point.

Without an embedding provider, or without a credential for the selected route, tool search falls back to BM25 automatically. `auto` and `jev` therefore cost nothing extra when no credential is set. A setup with no TinyHumans account needs an embedding provider that does not depend on one (for example `memory.embedding_provider = "custom:<OpenAI-compatible endpoint>"`) and a TypeSafe or OpenRouter key.

## Running the benchmark

`tool-search-bench` lives in the `profile/` crate of [openhuman-benchmarks](https://github.com/tinyhumansai/openhuman-benchmarks). From a checkout of that repository:

```bash
cargo run --manifest-path profile/Cargo.toml --bin tool-search-bench -- --ranker all --misses
cargo run --manifest-path profile/Cargo.toml --bin tool-search-bench -- --ranker jev --family --embedding
cargo run --manifest-path profile/Cargo.toml --bin tool-search-bench -- --dump-catalogue
```

`OPENHUMAN_BACKEND_API_KEY` (or `TYPESAFE_API_KEY`) selects the credential. Without one, the bench ranks through the signed-in TinyHumans session, as the product does.

## Where the code lives

- `vendor/tinyagents/vendor/tinytools/crates/tinytools-jev/`: the transport-neutral `ToolRanker` implementation, its retrieval strategies and the `JevRankerConfig` / `JevRanking` types.
- `crates/openhuman-tinyhumans/src/jev/`: the TinyHumans evaluator (`TinyJevEvaluator`, over `tinyjevclient`) and the process-installed ranker (`TinyHumansJevRanker`).
- `crates/openhuman-core/src/agent/tinyagents/discovery/`: the host slot that holds the installed ranker and turns `[agent.tool_search]` into the policy a turn runs with.
- `crates/openhuman-core/src/modules/browser_task.rs`: the browser-task entry point that runs `JevController`.
- [`docs/plans/jev-tool-search-baseline.md`](https://github.com/tinyhumansai/openhuman/blob/main/docs/plans/jev-tool-search-baseline.md): the full benchmark writeup.

See also [Performance](performance.md) for the wider cost and density picture, [Pluggable engines](engines.md) for the other swappable pieces, and [Embedding OpenHuman](embedding.md) for running an agent as a library.
