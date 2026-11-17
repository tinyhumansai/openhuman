---
description: >-
  Go from an empty Cargo project to many agents on one OpenHuman runtime in
  your own Rust process.
icon: rocket
---

# Build agents in Rust: quickstart

This page takes you from an empty Cargo project to a fleet of agents in one process. Each step builds on the last and ends with code you can run. [Embedding OpenHuman](embedding.md) is the full reference for every type used here.

## What you get

`openhuman-embed` runs the OpenHuman core inside your binary. There is no daemon to start and no RPC hop, so a turn is a function call. You build one `Runtime` per process, then create as many `Agent`s on it as you need. Each agent has its own model, access tier, working directory, MCP servers and skills.

Sharing one process is where the density comes from. The bootstrap cost is paid once, and each extra agent adds a small marginal cost. With a mock inference provider, 500 agents in one process used about 1.8 MiB of marginal memory each. Thousands of agents per box is a goal, not a measured result. The numbers and how to reproduce them are on the [Performance](performance.md) page.

## Prerequisites

You need Rust 1.99.1 or newer, the version pinned in [`rust-toolchain.toml`](https://github.com/tinyhumansai/openhuman/blob/main/rust-toolchain.toml). You also need an API key for any OpenAI-compatible endpoint, or a TinyHumans API key (see step 6).

If you depend on OpenHuman through Cargo as shown below, Cargo fetches the vendored submodules for you. If you build from a clone, initialize them first:

```bash
git submodule update --init --recursive vendor/
```

## Step 1: add the dependency

```toml
[dependencies]
openhuman-embed = { git = "https://github.com/tinyhumansai/openhuman", package = "openhuman-embed" }
# Needed for the tokio worker settings in step 2.
openhuman_core = { git = "https://github.com/tinyhumansai/openhuman", package = "openhuman" }
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync", "time"] }
anyhow = "1"
```

The default feature set is the contributor build. To ship less code, turn defaults off and pick the features you use. The set below keeps the model provider layer and MCP:

```toml
openhuman-embed = { git = "https://github.com/tinyhumansai/openhuman", package = "openhuman-embed", default-features = false, features = ["inference", "mcp"] }
```

Cargo applies `[patch]` tables only from the top-level workspace, so the ones in OpenHuman's [root `Cargo.toml`](https://github.com/tinyhumansai/openhuman/blob/main/Cargo.toml) do not reach your crate. They make every crate share one copy of `tinytools` and `tinyinference`. If dependency resolution fails over those crates, or you see two incompatible `tinytools` types, copy the same `[patch]` sections into your own workspace.

Each feature forwards to the same-named feature on the core. `mcp` and `skills` also gate `AgentSpec::mcp` and `AgentSpec::skills_dir`, so enable them if your agents need those.

## Step 2: build the tokio runtime yourself

Do not use `#[tokio::main]`. A turn is a large async state machine, and a sub-agent nested inside a turn overflows tokio's default 2 MiB worker stack and aborts the process. Build the runtime with the stack size and blocking-thread limit that the core exports:

```rust
use openhuman_core::core::runtime::{AGENT_WORKER_STACK_BYTES, MAX_BLOCKING_THREADS};

fn main() -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(AGENT_WORKER_STACK_BYTES)
        .max_blocking_threads(MAX_BLOCKING_THREADS)
        .build()?;
    rt.block_on(run())
}

async fn run() -> anyhow::Result<()> {
    Ok(())
}
```

The remaining steps fill in `run`.

## Step 3: your first agent

`Harness` is a runtime plus exactly one agent. It is the shortest path to a reply. This version points at an OpenAI-compatible endpoint with a key you supply, uses a throwaway workspace, and is read-only:

```rust
use openhuman_embed::{Access, Harness, Provider, Workspace};

async fn run() -> anyhow::Result<()> {
    let harness = Harness::builder()
        .provider(
            Provider::openai_compatible(
                "https://api.openai.com/v1",
                std::env::var("OPENAI_API_KEY")?,
            )
            .model("gpt-5"),
        )
        .workspace(Workspace::Ephemeral)
        .access(Access::readonly())
        .action_dir(std::env::current_dir()?)
        .build()
        .await?;

    let outcome = harness.run("What can you see in this directory?").await?;
    println!("{}", outcome.reply);
    Ok(())
}
```

Pass the API root as the URL. `/chat/completions` is appended for you. `Workspace::Ephemeral` is removed when the harness drops, so nothing touches an existing install. `Access::readonly()` allows no writes and no shell, which makes it safe to point at any directory.

A complete runnable version, with progress streaming, is the [`run_turn` example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/run_turn.rs):

```bash
OPENHUMAN_EXAMPLE_BASE_URL=https://api.openai.com/v1 \
OPENHUMAN_EXAMPLE_API_KEY=sk-... \
OPENHUMAN_EXAMPLE_MODEL=gpt-5 \
  cargo run -p openhuman-embed --example run_turn -- "What can you see in this directory?"
```

## Step 4: many agents on one runtime

When you need more than one agent, build the `Runtime` directly and create agents from `AgentSpec`s. `Harness` does this underneath, so moving over later is a small change.

```rust
use openhuman_embed::{Access, AgentSpec, Provider, Runtime, Workspace};

async fn run() -> anyhow::Result<()> {
    let runtime = Runtime::builder()
        .workspace(Workspace::dir("/var/lib/my-product/openhuman"))
        .build()
        .await?;

    let provider = Provider::openai_compatible(
        "https://api.openai.com/v1",
        std::env::var("OPENAI_API_KEY")?,
    )
    .model("gpt-5");

    let analyst = runtime.agent(
        AgentSpec::new("analyst")
            .system_prompt("You summarize documents and never edit files.")
            .provider(provider.clone())
            .access(Access::readonly())
            .action_dir("/srv/documents"),
    )?;

    let writer = runtime.agent(
        AgentSpec::new("writer")
            .system_prompt("You compose explanations inside your working directory.")
            .provider(provider)
            .access(Access::full())
            .action_dir("/srv/documents"),
    )?;

    let analysis = analyst.run("Summarise this document.").await?;
    let draft = writer
        .turn(format!("Explain these points:\n{}", analysis.reply))
        .send()
        .await?;
    println!("{}", draft.reply);
    Ok(())
}
```

Agent ids must match `^[a-z0-9][a-z0-9_-]{0,63}$`. Avoid the built-in ids such as `orchestrator`.

The access tier is the main safety control:

- `Access::readonly()` observes only.
- `Access::supervised()` can act, but risky operations wait for a human decision. It turns the autonomy policy on for that agent, so an unattended process stalls at the first approval until the ten-minute timeout denies it.
- `Access::full()` acts autonomously with real shell and file access under `action_dir`. Point it at a directory you are willing to have changed. Credential stores such as `~/.ssh` stay blocked regardless.

`action_dir` is the agent's read and write root. `.trust(path, access)` grants a directory outside it.

MCP servers and skills are per agent. They need the `mcp` and `skills` features, and the runtime must have those domains enabled because agents can only narrow the runtime's set:

```rust
use openhuman_embed::McpServer;

let spec = AgentSpec::new("triage")
    .mcp(McpServer::stdio("github", "gh-mcp", ["stdio"]).deny_tools(["delete_repo"]))
    .skills_dir("./skills/triage");
```

A server declared on one agent is invisible to the others. Skill bundles are copied into `<workspace>/agents/<id>/skills/`, and the operator's `~/.openhuman/skills` stays hidden unless you call `.include_user_skills(true)`. Narrowing which tools an MCP server exposes with `.allow_tools` or `.deny_tools` matters for large servers, because every exposed tool costs prompt budget.

The [`two_agents` example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/two_agents.rs) runs a analyst and a writer end to end.

## Step 5: multi-turn sessions and streaming

`agent.turn(message)` returns a builder. Each outcome carries a `session_id`; pass it back with `.session(..)` to continue the conversation. Without it, a new session is started.

```rust
let first = writer.run("Run the tests.").await?;
let again = writer
    .turn("Now fix the failures.")
    .session(&first.session_id)
    .send()
    .await?;
```

Transcripts are keyed by agent id, and a turn resumes only its own thread. With `Workspace::Ephemeral` sessions do not survive the process; use `Workspace::dir(..)` for that.

To watch a turn as it runs, attach a channel with `.on_progress(tx)`:

```rust
let (tx, mut rx) = tokio::sync::mpsc::channel(256);
let printer = tokio::spawn(async move {
    while let Some(progress) = rx.recv().await {
        eprintln!("[progress] {progress:?}");
    }
});
let outcome = writer.turn("go").on_progress(tx).send().await?;
let _ = tokio::time::timeout(std::time::Duration::from_secs(30), printer).await;
```

The core awaits every send, so a receiver that stops draining stalls the turn. The timeout covers detached sub-agents that can hold a clone of the sender after `send()` returns. Other per-turn options are `.model(id)`, `.temperature(t)`, `.cwd(dir)` and `.origin(..)`.

## Step 6: managed inference with one API key

Instead of a provider key for each agent, you can give the runtime one TinyHumans API key. Agents that name no `Provider` then run on managed inference, and backend features such as integrations and search use the same key. See [One TinyHumans API key](tinyhumans-api-key.md) for what it unlocks.

`openhuman-embed` alone installs no backend transport, so hosted surfaces answer `BACKEND_UNAVAILABLE:`. To boot connected, use the `RuntimeBuilder` from `openhuman-tinyhumans`, which mirrors the embed builder and installs the transport on `build()`:

```toml
openhuman-tinyhumans = { git = "https://github.com/tinyhumansai/openhuman", package = "openhuman-tinyhumans" }
```

```rust
use openhuman_tinyhumans::{embed::{Access, AgentSpec, Workspace}, RuntimeBuilder};

let runtime = RuntimeBuilder::new()
    .workspace(Workspace::Ephemeral)
    .api_key(std::env::var("TINYHUMANS_API_KEY")?)
    .build()
    .await?;

let agent = runtime.agent(AgentSpec::new("helper").access(Access::readonly()))?;
let reply = agent.run("Hello.").await?;
println!("{}", reply.reply);
```

For a headless process that boots the core itself, set `OPENHUMAN_BACKEND_API_KEY` before startup instead. An agent that names its own `Provider` never uses the key.

## Step 7: going to scale

A few rules keep a large fleet small and predictable:

- Run one `Runtime` per process. A second `build()` returns `RuntimeError::AlreadyRunning`, because the keyring, event bus and domain subscribers are process-wide. Add agents, not runtimes.
- Build narrow. A headless build with `--no-default-features --features "skills,flows"` is about 60 MiB stripped. With every gate on it is 115.9 MiB unstripped. The recipe is in [`docs/library-minimal-recipe.md`](https://github.com/tinyhumansai/openhuman/blob/main/docs/library-minimal-recipe.md).
- Narrow each agent. `.tool_groups(..)`, `.domains(..)`, `ToolScopeSpec::Named(..)` and `.disallow_tools(..)` cut prompt size and the surface an agent can reach.
- Pick a sandbox mode per agent through `AgentDefinitionSpec`: `SandboxModeSpec::None`, `ReadOnly` (write and execute tools are removed) or `Sandboxed` (commands run in the platform jail or Docker).
- For a cloud host serving many users from one process, use `Workspace::stateless()` with a `SessionStoreProvider` so conversations live in your own database. The embed crate README covers it under "Conversations in a host store".

To measure your own numbers, the scripts and method are in [openhuman-benchmarks](https://github.com/tinyhumansai/openhuman-benchmarks) ([`profile/docs/library-benchmarking.md`](https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/profile/docs/library-benchmarking.md)), alongside the public benchmark results and rig.

## Other ways in

You do not have to embed the library to use the core.

The JSON-RPC server runs the same core behind HTTP. Start it with `openhuman-core serve`. `GET /schema` lists every method, `GET /health` reports liveness and `GET /events` streams events. The server lives in [`crates/openhuman-rpc`](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-rpc/README.md).

The CLI is the `openhuman-core` binary. It exposes the same controllers as subcommands and is documented in [`crates/openhuman-cli`](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-cli/README.md).

The terminal client is `openhuman-tui`, a ratatui front end that boots the core in its own process and drives the same chat surface as the desktop app. See [`crates/openhuman-tui`](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-tui/README.md).

## Next steps

- [Embedding OpenHuman](embedding.md) is the full reference for `AgentSpec`, `Access`, workspaces, MCP, skills and the examples.
- [Architecture](architecture.md) and the [agent harness](architecture/agent-harness.md) explain how a turn runs.
- [Loadable modules](loadable-modules.md) covers the module contracts the core composes.
- [Pluggable engines](engines.md) covers the inference, memory and search backends an agent chooses by config.
- [Performance and footprint](performance.md) has the measured density, cold-start and binary-size numbers.
