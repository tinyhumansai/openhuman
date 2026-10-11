# Import ownership cleanup

Tracking: [OpenHuman #7408](https://github.com/tinyhumansai/openhuman/issues/7408).

The twelve shim files named in the issue have been removed. Callers import the
owning crate or domain directly. The wallet tool module now owns a concrete
six-tool factory rather than forwarding historical names. `tools/mod.rs` lists
explicit domain factories; it no longer glob-reexports domain tools. The ordered
184 tool constructor sites match the pre-change registration order.

Generic agent goals, todos, session vocabulary, cancellation, budgets,
artifact helpers, tool types, skill metadata, flow catalog records, wallet
metadata, pairing helpers and event subscription types now come from their
owner crates. The TinyJuice type facade was removed; its serialization tests
live beside the host module and import the bus contracts directly.

Existing public embed contracts keep their names while naming the owner types
directly. The boundary checker allows only the existing declarations and rejects
additional members. Embed's direct dependencies added here are the existing
TinyAgents harness and TinyBus runtime, with default features disabled. It does
not acquire implementation dependencies for loadable components.

## Remaining component boundaries

This cleanup does not complete the component migrations tracked by the other
children of the epic. Their remaining implementation facades require released
bus contracts and host adoption in the owning migration:

| Remaining boundary                                     | Owning work         |
| ------------------------------------------------------ | ------------------- |
| TinyBox jail and shell classification                  | #7391, #7392        |
| TinyFlows builder, checkpoints and schedule vocabulary | #7395, #7396, #7397 |
| Channel runtime and provider implementations           | #7412               |
| MCP transport clients and registry adapters            | #7416               |
| Wallet protocol and ledger implementation              | #7406               |

The always-on wallet error sentinel stays available through the core until a
released TinyWallet bus exposes it. RPC and observability classify that producer
error even in feature-disabled builds. Giving embed a wallet implementation
dependency would violate the component boundary. Schedule vocabulary likewise
stays available to the existing embed API until the schedule bus migration is
available.

The memory API, module contracts, storage ports, configuration contracts and
host controller/tool adapters remain distinct from these implementation
facades. Their public host operations are not generic implementation aliases.
An upstream bus vocabulary change must land in its owner before its host adapter
can consume it.

## Verification

The tool registry suite passed all 53 tests. The embed boundary suite passed all
five tests. The runtime boundary baseline fell from 194 to 181 exact existing
violations without allowing new violations. Rust layout pins only decrease.
Fresh root, desktop and feature-disabled build results are recorded in the PR;
this document does not substitute for those checks.

The issue's `scripts/e2e-core.sh` command does not exist in this checkout. A
missing runner is not an E2E pass. Composition tests must use the repository's
current mock-backed runner and report its actual results.
