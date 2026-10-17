# openhuman-cli

This crate builds the `openhuman-core` binary, two ops binaries
(`openhuman-fleet`, `test-mcp-stub`), and every root `tests/*.rs` and
`examples/*.rs` target in the repository. The core ([`crates/openhuman-core`](../openhuman-core/), package `openhuman`) is a
library with no backend client and no JSON-RPC server, and it declares no
bin, test or example targets of its own. Anything that must run as a process
against the hosted backend, or test the core the way a host boots it, lives
here.

## How it works

### Where it sits

```text
   openhuman-cli  (openhuman-core binary, ops bins)
        |
        v   the only normal OpenHuman dependency
   openhuman-rpc  (host::cli, JSON-RPC server, session store)
        |
        v
   openhuman-tinyhumans  (SDK transport, hosted proxies, session owner)
        |
        v
   openhuman-embed  (Runtime/RuntimeBuilder, process helpers, facades)
        |
        v
   openhuman-core  (domains, CLI dispatcher, controller registry)
```

The binary depends on `openhuman-rpc` alone
(`scripts/ci/check-crate-chain.mjs` enforces it), and every feature gate
forwards to `openhuman-rpc/<gate>`. The root tests and examples still reach
into the core: `openhuman-core`, `openhuman-embed` and `openhuman-tinyhumans`
are **dev-dependencies**, so they are compiled into test and example targets
and never into the shipped binary.

### Startup of `openhuman-core`

[`src/main.rs`](src/main.rs) is short and runs in a fixed order:

1. `restore_default_sigpipe()` resets `SIGPIPE` to the default on unix, so
   piping output into `head` ends the process quietly instead of panicking
   on `EPIPE`.
2. `embed::process::load_dotenv_for_cli()` loads `.env` (or
   `OPENHUMAN_DOTENV_PATH`) before Sentry starts, so a DSN defined only there
   is visible. Variables already in the environment win.
3. With the `crash-reporting` feature, Sentry starts with
   `embed::process::sentry::client_options`: the DSN from
   `OPENHUMAN_CORE_SENTRY_DSN`, then `OPENHUMAN_SENTRY_DSN`, at runtime and
   then at compile time; the release tag `openhuman@<version>[+<sha>]`; and
   embed's shared `before_send` chain (the same one the desktop shell and the
   TUI install), which drops known-noise classes, strips `server_name`, falls
   back to the credential identity for the user id, and scrubs secrets.
4. `openhuman_rpc::host::cli(&args)` does the rest: it connects the
   TinyHumans backend (SDK transport, hosted RPC proxies, Jev ranker), puts
   the JSON-RPC server behind `run` / `serve`, registers the `http_host`
   controllers, and runs the core's dispatcher. A failure exits with
   status 1.

### Subcommands

The dispatcher is `openhuman_core::core::cli::run_from_cli_args`. Global
options go before the command: `-m`/`--model` (alias `--model-id`) and
`-p`/`--provider` (alias `--provider-id`) set process-local overrides that are
never written back to the config file.

| Command | What it does |
| --- | --- |
| `run`, `serve` | Start the HTTP JSON-RPC and Socket.IO server. Flags: `--host`, `--port`, `--jsonrpc-only` (no Socket.IO), `--headless-api` (JSON-RPC only, no background services), `-v`/`--verbose`. |
| `call --method <name> [--params '<json>' \| --params-stdin]` | Invoke one controller in-process (no server needed) and print the JSON result. Runs on a runtime with the agent worker stack size, so methods that run a turn work. |
| `mcp`, `mcp-server` | Run the stdio MCP server. No banner, since stdout carries JSON-RPC. |
| `agent <dump-prompt \| dump-all \| prompt-size \| list>` | Inspect agent definitions and rendered prompts (`core/agent_cli.rs`), for example `agent dump-prompt --agent <id> --json --with-tools`. |
| `sentry-test [--message <text>] [--panic]` | Send a test event to verify Sentry wiring. Reports a disabled-build error without `crash-reporting`. |
| `tui`, `chat` | Print a migration notice: the terminal UI is the separate `openhuman-tui` executable. |
| `<namespace> <function> [--param value ...]` | Generic dispatch to any registered controller, for example `skills ...` or `voice ...`. `<namespace> --help` lists functions; `<namespace> <function> --help` lists parameters. |

With no arguments or `--help`, it prints usage and the registered namespaces.
The banner goes to stderr so stdout stays clean JSON.

### Tests and examples are targets of this crate

Cargo only auto-discovers [`tests/`](../../tests/) and [`examples/`](../../examples/) beside the manifest, and
the files live at the repository root. So the manifest turns discovery off
(`autotests = false`, `autoexamples = false`, `autobins = false`,
`autobenches = false`) and declares each target explicitly with a path back
to the root:

```toml
[[test]]
name = "json_rpc_e2e"
path = "../../tests/json_rpc_e2e.rs"
required-features = ["voice"]
```

Every new `tests/<name>.rs` or `examples/<name>.rs` file needs such an entry.
`pnpm rust:layout` ([`scripts/ci/check-openhuman-rust-layout.mjs`](../../scripts/ci/check-openhuman-rust-layout.mjs)) fails on a
missing or stale entry, and on any target table in the core manifest.

Two targets aggregate whole directories, so files there need no entry. The
shared root [`build.rs`](../../build.rs) (wired with `build = "../../build.rs"`) globs them into
generated module lists:

- `raw_coverage_all` ([`tests/raw_coverage_all.rs`](../../tests/raw_coverage_all.rs)) includes every
  `tests/raw_coverage/*.rs` suite as a module. It needs `voice` and
  `inference`.
- `in_process_all` ([`tests/in_process_all.rs`](../../tests/in_process_all.rs)) includes every
  `tests/in_process/*.rs` suite. These boot the core router in the test
  process and are gate-free.

Folding the suites into one binary each links the large `openhuman` rlib once
instead of once per file. Suites that need a process of their own (global
`OnceCell`s, a real keyring, spawning the binary) stay separate targets.

A target that names symbols behind a product gate carries
`required-features`, so a contributor `cargo test` skips it instead of failing
to compile. The CI product lanes pass `--features
"$(scripts/ci/product-features.sh)"`, which turns them back on. Current gated
targets: `observability_smoke` (`crash-reporting`), `computer_bali_live_e2e`
(`modules`), `x402_twit_sh_live` (`web3`), `json_rpc_e2e` (`voice`),
`raw_coverage_all` (`voice`, `inference`) and `media_generation_e2e`
(`media`).

In-process suites that reach the (mock) backend call
`tinyhumans_boot::boot()` from [`tests/support/tinyhumans_boot.rs`](../../tests/support/tinyhumans_boot.rs) first, which
runs `openhuman_tinyhumans::install` (a dev-dependency here). Without it every backend call answers
`BACKEND_UNAVAILABLE:`. Suites that spawn the `openhuman-core` binary get the
transport from `main.rs`.

## Layout

| Path | What it does |
| --- | --- |
| [`Cargo.toml`](Cargo.toml) | All `[[bin]]`, `[[test]]` and `[[example]]` targets, feature forwarding. |
| [`src/main.rs`](src/main.rs) | The `openhuman-core` binary entry point described above. |
| [`src/bin/`](src/bin/README.md) | Ops binaries: `test-mcp-stub`, `openhuman-fleet`. The benchmark binaries live in [openhuman-benchmarks](https://github.com/tinyhumansai/openhuman-benchmarks) (`profile/`, #6944). |
| `../../tests/*.rs` | 27 `[[test]]` targets, including the two aggregators. See [`tests/README.md`](../../tests/README.md). |
| [`../openhuman-embed/examples/`](../openhuman-embed/examples/README.md) | Offline embedding examples, built and run as `openhuman-embed` targets. |
| `../../build.rs` | Shared build script: generates the `raw_coverage_all` and `in_process_all` module lists and exports `OPENHUMAN_REPOSITORY_ROOT`. |

## Targets

| Target | Source | Required features |
| --- | --- | --- |
| `openhuman-core` | `src/main.rs` | none |
| `test-mcp-stub` | [`src/bin/test_mcp_stub.rs`](src/bin/test_mcp_stub.rs) | none |
| `openhuman-fleet` | [`src/bin/fleet.rs`](src/bin/fleet.rs) | `http-server`, `bin-tools` |

## Features

The default set is the core's contributor default (listed by name, because
`openhuman-rpc`'s own default carries only its server and client) plus
`jev`. Every gate (`http-server`, `inference`, `voice`, `web3`, `storage-*`,
`channels`, `media`, `modules`, `e2e-test-support`, and the rest) forwards to
`openhuman-rpc/<gate>`, which forwards it down the chain, so the product
lanes' feature list resolves here unchanged;
[`scripts/ci/check-feature-forwarding.mjs`](../../scripts/ci/check-feature-forwarding.mjs) checks the link. `rss-bench` is
not forwarded, since the benchmark crate that uses it enables it on the core
directly. Gates local to this crate:

| Feature | Purpose |
| --- | --- |
| `bin-tools` | `clap` for `openhuman-fleet`. |

## Boundaries

- Subcommand parsing and dispatch live in the core
  ([`crates/openhuman-core/src/core/cli.rs`](../openhuman-core/src/core/cli.rs), `core/agent_cli.rs`). New
  functionality is a controller registered in `core/all.rs`, reached through
  the generic namespace dispatch, not a new branch in `cli.rs`.
- The JSON-RPC server, Socket.IO and the listener belong to
  [`crates/openhuman-rpc`](../openhuman-rpc/). The backend transport and login belong to
  [`crates/openhuman-tinyhumans`](../openhuman-tinyhumans/).
- The terminal UI is [`crates/openhuman-tui`](../openhuman-tui/); the desktop host is
  [`crates/openhuman-app`](../openhuman-app/). Both are hosts on `openhuman-rpc` like this one and
  do not use this binary (the app's `core` subcommand runs the same
  `host::cli`).
- The binary never names the core, embed or tinyhumans directly; reach new
  core behavior through embed's facade (re-exported as
  `openhuman_rpc::embed`).

## Gotchas

- Sentry's `before_send` filters are defense in depth. The primary
  suppression for each noise class lives at its emit site in the core; add
  new filters there first.
- The `[[test]]` comment block in [`Cargo.toml`](Cargo.toml) about product-gated targets
  still says autodiscovery stays on for the other targets. It does not:
  `autotests = false`, and every target is declared.
- Do not export `CARGO_TARGET_DIR`; the repository already configures a
  shared target directory.

## Tests

```bash
cargo build -p openhuman-cli --bin openhuman-core
cargo test  -p openhuman-cli --test in_process_all
cargo test  -p openhuman-cli --test json_rpc_e2e --features "$(bash scripts/ci/product-features.sh)"
pnpm test:rust          # scripts/test-rust-with-mock.sh, the canonical runner
pnpm debug rust <filter>
```

`main_tests.rs` (built with `crash-reporting`) covers the environment and
release resolution, and pins that the shared chain still scrubs secrets.

## Further reading

- [`docs/gitbooks/en/developing/building-rust-core.md`](../../docs/gitbooks/en/developing/building-rust-core.md): building the Rust core.
- [`docs/gitbooks/en/developing/testing-strategy.md`](../../docs/gitbooks/en/developing/testing-strategy.md): testing strategy.
- [`docs/gitbooks/en/developing/performance.md`](../../docs/gitbooks/en/developing/performance.md): performance.
- [`crates/README.md`](../README.md): crates overview.
- [`crates/openhuman-core/README.md`](../openhuman-core/README.md): the openhuman-core crate README.
- [`tests/README.md`](../../tests/README.md): root tests.
