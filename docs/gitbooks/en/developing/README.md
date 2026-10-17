---
description: Build, run, test and ship OpenHuman from source.
icon: code-branch
---

# Overview

OpenHuman is open source under GPLv3 at [github.com/tinyhumansai/openhuman](https://github.com/tinyhumansai/openhuman). This section is for contributors and anyone running OpenHuman from source.

If you just want to use the app, start with [Getting started](../overview/getting-started.md). If you want to read the architecture, build a feature or land a PR, you are in the right place.

---

## Why the harness is light

The core is a Rust library, not a set of services talking over sockets. Agents run in-process, so a turn costs a function call instead of a process boundary or an RPC hop. We measured 500 agents alive at once in one process at roughly 1.77 MiB of marginal memory each. A cold agent turn takes about 100 ms, and a stripped build with only the domains you need can be as small as 51 MiB. [Performance](performance.md) has the numbers and how we got them. The rig that produces them is public at [tinyhumansai/openhuman-benchmarks](https://github.com/tinyhumansai/openhuman-benchmarks).

Everything above that floor is modular. Cargo feature gates decide what compiles in, loadable native modules carry the heavier engines, and config chooses the engines themselves: model providers, embeddings, memory and search. [Pluggable engines](engines.md), [Loadable modules](loadable-modules.md) and [Jev](jev.md) cover that.

## Where things live

| Path        | What is there |
| ----------- | ------------- |
| `app/`      | The pnpm workspace `openhuman-app`: the Vite and React frontend ([`app/src/`](https://github.com/tinyhumansai/openhuman/tree/main/app/src)) and the Tauri desktop host ([`crates/openhuman-app/`](https://github.com/tinyhumansai/openhuman/tree/main/crates/openhuman-app)). |
| `crates/`   | Rust crates ([overview](https://github.com/tinyhumansai/openhuman/blob/main/crates/README.md)). Six are workspace members. `openhuman-core` (package `openhuman`) is the core library, with domains under `src/<domain>/` and the controller contract and in-process dispatch under `src/core/`. It has no binary and no JSON-RPC server. `openhuman-rpc` is JSON-RPC 2.0 over the core: envelopes, HTTP client and server. `openhuman-embed` is the typed library facade. `openhuman-tinyhumans` is the only crate that may depend on `tinyhumans-sdk`, and holds the backend transport, hosted RPC proxies and host-side session owner. Every host installs it first. `openhuman-cli` is the `openhuman-core` binary plus every root `tests/` and `examples/` target. `openhuman-tui` is the terminal frontend. The Tauri host, `openhuman-app`, is excluded from the root workspace and builds from its own manifest. |
| `docs/gitbooks/en/` | This site, the public docs ([source](https://github.com/tinyhumansai/openhuman/tree/main/docs/gitbooks)). |
| `docs/`     | Internal maintainer docs: test-coverage matrix, release smoke checklist, library benchmarking and minimal-recipe notes, translated READMEs and `community/` ([`docs/`](https://github.com/tinyhumansai/openhuman/tree/main/docs)). |

[`AGENTS.md`](https://github.com/tinyhumansai/openhuman/blob/main/AGENTS.md) at the repo root is the source of truth for AI agents working on the codebase. [`CLAUDE.md`](https://github.com/tinyhumansai/openhuman/blob/main/CLAUDE.md) points to it. The same rules apply to humans.

---

## Start here

If this is your first time in the repo:

1. [Getting set up](getting-set-up.md): toolchain, dependencies, the Tauri CLI and everything `pnpm dev` needs.
2. [Building the Rust core](building-rust-core.md): setup for the Rust workspace only, with the pinned toolchain, OS packages and exact `cargo` commands.
3. [Architecture](architecture.md) and its [overview](architecture/README.md): how the desktop app, the in-process Rust core, the JSON-RPC bridge and the dual sockets fit together. Read this before non-trivial changes.
4. [Frontend](architecture/frontend.md) and [Tauri shell](architecture/tauri-shell.md): the React app and the desktop host that wraps it.
5. [MCP server](mcp-server.md): the opt-in stdio MCP mode that exposes OpenHuman search, memory and sub-agent tools to local clients.

---

## The core in depth

- [Performance](performance.md): memory per agent, cold-start timing, binary size and how the benchmarks run. The raw rigs and results are in the public [openhuman-benchmarks](https://github.com/tinyhumansai/openhuman-benchmarks) repo.
- [Pluggable engines](engines.md): how config chooses the LLM, embeddings, memory and web search layers, and what is available for each.
- [Jev](jev.md): the fast probability model behind tool selection and routine browser decisions.
- [Embedding OpenHuman](embedding.md) (see also [`crates/openhuman-embed`](https://github.com/tinyhumansai/openhuman/tree/main/crates/openhuman-embed)): using `openhuman-embed` to run the core, and any number of agents on it, inside another Rust product. The [Rust quickstart](quickstart.md) walks through it.
- [SaaS profiles](saas-profiles.md): running the core as a multi-user service behind a gateway, one profile per user, on one node or a cluster with leased profiles.
- [One TinyHumans API key](tinyhumans-api-key.md): what a single key unlocks, including managed inference, embeddings, web search, media, integrations, voice and Jev.

---

## Testing

OpenHuman has three test layers. Know which one your change belongs in. The Rust integration targets are described in [`tests/README.md`](https://github.com/tinyhumansai/openhuman/blob/main/tests/README.md) and the helper scripts in [`scripts/README.md`](https://github.com/tinyhumansai/openhuman/blob/main/scripts/README.md).

- [Testing strategy](testing-strategy.md): when to write Vitest, cargo or WDIO tests.
- [E2E testing](e2e-testing.md): WDIO and Appium specs, the Linux and macOS setups, and how to run a single spec locally.
- [Agent observability](agent-observability.md): the artifact capture that makes E2E and agent runs debuggable after the fact.

PRs must reach at least 80% coverage on changed lines. Add tests for new behavior, not just the happy path.

---

## Shipping

- [Release policy](release-policy.md): versioning, release cadence, OAuth and installer rules.
- [Cloud deploy](../features/cloud-deploy.md): deployment when a change crosses the desktop boundary.

---

## Going deeper

- [Agent harness](architecture/agent-harness.md): the tinyagents turn loop (checkpointing, circuit breakers, sub-agent handback, journals and replay) and how to extend the tool surface.
- [Workflows](../features/workflows.md): the tinyflows-backed `flows` domain, with triggers, trust origins, approval-gated runs and the `flows_*` RPC surface.
- [Loadable modules](loadable-modules.md): native `cdylib` modules the core loads at runtime, and how they are admitted.
- [Hooks](hooks.md): scripts you own that run before a tool executes, after a file edit or when a turn finishes.
- [Chromium Embedded Framework](cef.md): design notes from the CEF era. The shell now runs on stock Tauri (Wry).

---

## Contributing

- Open issues and PRs at [tinyhumansai/openhuman](https://github.com/tinyhumansai/openhuman). The [`vendor/`](https://github.com/tinyhumansai/openhuman/tree/main/vendor) submodules (tinyagents, tinymemory and others) have their own repos under [tinyhumansai](https://github.com/tinyhumansai).
- PRs target `main`. Push to your fork, not upstream.
- Follow [`CONTRIBUTING.md`](https://github.com/tinyhumansai/openhuman/blob/main/CONTRIBUTING.md) and the issue and PR templates.
- Keep changes focused. A bug fix does not need surrounding cleanup.

Building toward AGI does not have to mean shipping a kernel. Bug fixes, docs, integrations and tests all move the bar.
