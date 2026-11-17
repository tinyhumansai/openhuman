# jev

Installs Jev as the core's `tool_search` ranker in a TinyHumans-connected
process. The agent harness exposes a `tool_search` bridge over every deferred
tool and ranks searches with whichever `ToolRanker` the process installed.
This module provides `TinyHumansJevRanker`: an embedding retriever cuts the
catalogue to a shortlist, and one Jev `Choice` evaluation picks from it. The
module compiles only with the crate's `jev` feature (on by default).

See [`docs/gitbooks/en/developing/jev.md`](../../../../docs/gitbooks/en/developing/jev.md) for
what Jev is, the benchmark numbers, and how it compares to plain BM25.

## How it works

`crate::install()` calls `install_jev_ranker()`, which builds a
`TinyHumansJevRanker` and registers it through
`openhuman_embed::__host::agent::tinyagents::discovery::install_tool_ranker`. Calling
it again replaces the ranker in place. Install needs no login, because the
ranker resolves everything per search.

On each `rank` call:

```text
 tool_search(intent, candidates)
   |
   v
 load config (embedder's bound config, else process-global)
   |
   v
 route::resolve(config, env)    agent.tool_search.jev_route
   |   tinyhumans | typesafe | openrouter | auto (tries them in that order)
   |   error? -> RankError; the harness answers with its own BM25
   v
 fingerprint(route + secret, base_url)
   |   same as cached?  -> reuse the cached JevRanker
   |   changed/none     -> build tinyjevclient::Client
   |                       + TinyJevEvaluator (deadline)
   |                       + retriever (embedding ranker, reused on rebuild)
   v
 JevRanker::rank_detailed -> hits (debug log: families, confidence,
                                   needs_tool, latency, tokens)
```

Reading config and credential on every search costs a config load, which a
`tool_search` round trip dwarfs. In return, sign-in, sign-out and a backend
URL change all take effect on the next search. The client cache key is a hash
of the route label, secret and base URL; the secret itself is never logged.

### Routes (`route.rs`)

Jev is reachable three ways, and the route is an operator decision
(`agent.tool_search.jev_route`), so decision calls never go somewhere the
operator did not choose:

| Route | Credential | Base URL |
| --- | --- | --- |
| `tinyhumans` | the core's `resolve_backend_credential` (session JWT or API key) | `effective_backend_api_url`, through the backend's `/agent-integrations/openrouter/systemone` proxy |
| `typesafe` | `TYPESAFE_API_KEY` | the client default, or `agent.tool_search.jev_base_url` |
| `openrouter` | `OPENROUTER_API_KEY`, else the stored `openrouter` provider key | the client default, or `jev_base_url` |
| `auto` | tries `tinyhumans`, then `typesafe`, then `openrouter` | from whichever wins |

An unknown spelling parses as `auto` with a warning, so a typo does not turn
tool ranking off. When every candidate route fails, the error lists each gap
(never a secret) so the BM25 fallback's log line says what to set.

### Retrieval

`retriever_for` uses the process's default embedding provider (the same one
memory recall uses) wrapped as an embedding tool ranker, with a disk cache at
`<workspace>/cache/tool_search_embeddings.json`. The retriever survives a
credential change, so the catalogue is embedded once per process. A provider
that cannot embed (`none`) is an error rather than a BM25 substitute: a Jev
decision over a lexical shortlist would cap Jev at BM25's recall and only add
a network round trip. In that case the harness answers with its own BM25
catalogue.

### Evaluation (`evaluator.rs`)

`TinyJevEvaluator` implements `tinytools_jev::JevEvaluator` over
`tinyjevclient`. `tinytools-jev` decides what to ask (options, intent,
family-stage wording); the evaluator owns the wire. Each request carries two
questions: a `Choice` named `tool` over the shortlisted options, and a `Noul`
named `needs_tool` asking whether the request needs a tool at all. The
evaluator applies its own deadline so a slow answer becomes a fallback rather
than a stalled turn. The ranker sets a 6 s per-evaluation deadline (measured
evaluations through the proxy run 0.7 to 1.9 s at p50); the evaluator's own
default is 3 s.

## Layout

| Path | What it does |
| --- | --- |
| [`mod.rs`](mod.rs) | `install_jev_ranker()` and the re-exports. |
| [`ranker.rs`](ranker.rs) | `TinyHumansJevRanker`: per-search config and route resolution, the client cache, the embedding retriever, the `ToolRanker` impl. Test seams: `with_config_loader`, `with_env_loader`, `with_deadline`, `with_config`. |
| [`route.rs`](route.rs) | `JevRoute`, `resolve`, `TYPESAFE_API_KEY_ENV`, `OPENROUTER_API_KEY_ENV`. |
| [`evaluator.rs`](evaluator.rs) | `TinyJevEvaluator`, the System One wire. |

## Key types and entry points

- `install_jev_ranker()` ([`mod.rs`](mod.rs)): what `install()` calls.
- `TinyHumansJevRanker` ([`ranker.rs`](ranker.rs)): the ranker; `kind()` reports
  `JevRanker::KIND`.
- `JevRoute` ([`route.rs`](route.rs)): the parsed `jev_route` setting.
- `TinyJevEvaluator` ([`evaluator.rs`](evaluator.rs)): reusable on its own with any
  `tinyjevclient::Client`.

## Boundaries

- The generic ranker types (`JevRanker`, `JevEvaluator`, `JevStrategy`,
  `JevRankerConfig`) live in
  [`vendor/tinyagents/vendor/tinytools/crates/tinytools-jev`](../../../../vendor/tinyagents/vendor/tinytools/crates/tinytools-jev/)
  (`tinyhumansai/tinytools`). Changes to how Jev is asked, or to family
  staging, belong there.
- The `ToolRanker` trait is `tinytools`'. The `tool_search` bridge and its
  BM25 fallback are the harness's (`tinyagents`), adapted in the core's
  `agent::tinyagents::discovery`.
- `tinyjevclient` is consumed by pinned git revision; this module owns only
  the credential, base URL and deadline.
- Config lives in the core: `ToolSearchConfig` in
  [`crates/openhuman-core/src/config/schema/agent.rs`](../../../openhuman-core/src/config/schema/agent.rs) (`ranker` is `jev` by
  default, or `auto`, `bm25`, `compare`; plus `jev_route` and
  `jev_base_url`).

## Tests

[`ranker_tests.rs`](ranker_tests.rs), [`route_tests.rs`](route_tests.rs) and [`evaluator_tests.rs`](evaluator_tests.rs) sit beside their
modules and use the config and env seams rather than the process
environment. The ranker comparison harness (`tool-search-bench`) moved out of
`openhuman-cli` with the other benchmarks (#6944), to the `profile/` crate of
[openhuman-benchmarks](https://github.com/tinyhumansai/openhuman-benchmarks)
(`cargo run --manifest-path profile/Cargo.toml --bin tool-search-bench`).

```bash
cargo test -p openhuman-tinyhumans jev::
```

## Further reading

- [`docs/gitbooks/en/developing/jev.md`](../../../../docs/gitbooks/en/developing/jev.md): Jev.
- [`docs/gitbooks/en/developing/performance.md`](../../../../docs/gitbooks/en/developing/performance.md): performance.
- [`crates/openhuman-tinyhumans/README.md`](../../README.md): the openhuman-tinyhumans crate README.
