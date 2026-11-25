# bin

Auxiliary binaries declared as `[[bin]]` targets in
[`crates/openhuman-cli/Cargo.toml`](../../Cargo.toml), next to the primary `openhuman-core`
binary ([`src/main.rs`](../main.rs), described in the [crate README](../../README.md)). One
is a test fixture and the other an experimental multi-tenant supervisor.
Neither ships in the desktop product, and neither names an OpenHuman crate:
the CLI's normal dependencies stop at `openhuman-rpc`.

The benchmark and profiling binaries that used to live here
(`tool-search-bench`, `tool-dialect-bench`, `rss-bench`, `library-profile`)
reached deep into core internals (`agent::harness`, `platform::proc_metrics`,
`flows`, provider factories) that the curated facade does not expose. They
moved, with their `scripts/profile/` drivers, to the `profile/` crate of
[openhuman-benchmarks](https://github.com/tinyhumansai/openhuman-benchmarks) (#6944), which builds them against a vendored
checkout of this workspace; the `rss-bench` / `rss-bench-dhat` gates went
with them.

## How it works

Each binary is a separate `[[bin]]` entry (`autobins = false`), and
`openhuman-fleet` carries `required-features` so a plain build skips it
instead of failing to link:

| Binary | Source | Required features | Purpose |
| --- | --- | --- | --- |
| `test-mcp-stub` | [`test_mcp_stub.rs`](test_mcp_stub.rs) | none | Minimal stdio MCP server that tests spawn. |
| `openhuman-fleet` | [`fleet.rs`](fleet.rs) | `http-server`, `bin-tools` | Process-per-user supervisor and reverse proxy. |

`http-server` is in `default`; `bin-tools` is not. A plain
`cargo build -p openhuman-cli` therefore produces `openhuman-core` and
`test-mcp-stub`.

### test-mcp-stub

Speaks just enough MCP to answer `initialize`, `tools/list` and `tools/call`
for one `echo` tool, over newline-delimited JSON-RPC on stdin and stdout, and
exits when stdin closes. `initialize` reports `PROTOCOL_VERSION`
(`2025-11-25`). It depends on nothing beyond `serde_json`. Tests spawn it
through `env!("CARGO_BIN_EXE_test-mcp-stub")`, which makes Cargo build it for
every test run: [`tests/mcp_registry_e2e.rs`](../../../../tests/mcp_registry_e2e.rs),
[`tests/mcp_registry_multi_server.rs`](../../../../tests/mcp_registry_multi_server.rs), [`tests/agent_harness_e2e.rs`](../../../../tests/agent_harness_e2e.rs),
[`tests/json_rpc_e2e.rs`](../../../../tests/json_rpc_e2e.rs), [`tests/in_process/domain_modules_e2e.rs`](../../../../tests/in_process/domain_modules_e2e.rs) and
[`tests/raw_coverage/tool_registry_approval_e2e.rs`](../../../../tests/raw_coverage/tool_registry_approval_e2e.rs).

### openhuman-fleet

Hosts one `openhuman-core` process per user or workspace behind a single
endpoint, so a team server can run many members' assistants while every
existing client (`CloudHttpTransport`) keeps working unchanged. The design is
process-per-user, not in-process multi-tenancy:

```text
 client --POST /{user_id}/rpc + edge token--> openhuman-fleet (--listen)
                                                 |  checks EdgeToken
                                                 |  swaps in CoreBearer
                                                 v
                    openhuman-core run --headless-api --port base+N
                    OPENHUMAN_WORKSPACE=<workspaces_root>/<user>
                    OPENHUMAN_CORE_TOKEN=<core bearer>
```

- Each tenant is its own OS process with its own workspace volume and its own
  core bearer. Tenants do not yet run under distinct OS users or containers,
  so this MVP is not a production multi-tenant security boundary for
  arbitrary agent tools.
- The supervisor mints a distinct edge token per tenant for clients and is
  the only holder of the tenants' core bearers. `EdgeToken` and `CoreBearer`
  are separate newtypes so they cannot be confused. Minted edge tokens are
  written to the file named by `--edge-token-output`.
- The proxy forwards `POST /{user_id}/rpc` verbatim to
  `http://127.0.0.1:<port>/rpc`, so the JSON-RPC wire contract is unchanged
  end to end.

Flags: `--listen` (default `127.0.0.1:8899`), `--workspaces-root` (default
`./fleet-workspaces`), `--core-bin` (default `openhuman-core`),
`--base-core-port` (default 7900; tenant N listens on base + N), `--users`
(comma-separated ids to provision at boot) and `--edge-token-output`
(required). Ports are assigned sequentially, and each tenant must pass an
authenticated JSON-RPC readiness probe before it is registered. A production
supervisor would read each core's bound port from a ready file
(`EmbeddedReadySignal`) and reconcile membership against
`tinyhumansai/backend`.

```bash
cargo build -p openhuman-cli --features bin-tools --bin openhuman-fleet
```

## Boundaries

- The `openhuman-core` entry point is [`src/main.rs`](../main.rs), one directory up, not
  here.
- Benchmarks and profiling drivers live in [openhuman-benchmarks](https://github.com/tinyhumansai/openhuman-benchmarks);
  the pure RSS sampling code they use stays in the core
  (`platform::proc_metrics`).

## Gotchas

- The manifest sets `autobins = false`, so a `.rs` file in this directory is
  a binary only when it has a `[[bin]]` entry. That is what lets
  [`fleet_tests.rs`](fleet_tests.rs) sit here beside its binary
  without Cargo trying to build it as an executable. A new binary needs both
  the file and the manifest entry.

## Tests

[`fleet_tests.rs`](fleet_tests.rs) (port assignment, user scoping, provisioning, bearer parsing)
sits beside its binary and builds with its required features.
`test-mcp-stub` is exercised by the MCP suites listed above.

```bash
cargo test -p openhuman-cli --features bin-tools --bin openhuman-fleet
```

## See also

- [openhuman-benchmarks](https://github.com/tinyhumansai/openhuman-benchmarks): the benchmark and profiling binaries
  (`profile/`), their driver scripts and the method behind the published
  numbers.
- [`docs/gitbooks/en/developing/performance.md`](../../../../docs/gitbooks/en/developing/performance.md):
  the numbers those benchmarks feed.
