# web3_swap

Single-chain token swaps use the TinyWallet module for provider parsing, quote
construction, module-owned quote storage, transaction encoding, approval-bound
signing, and broadcast. OpenHuman keeps the `web3_swap` RPC controllers and
agent-tool adapters, and supplies authenticated access to the backend
deBridge proxy plus generic, guarded chain transport.

## Operations

| Function | Purpose |
| --- | --- |
| `quote` | Prepare a single-chain swap quote. Cross-chain requests are routed to `web3_bridge`. |
| `execute` | Execute a stored quote after `confirmed: true` and host approval of the exact intent. |
| `routes` | List provider-supported routes through the module/backend boundary. |

Quotes are bound to their preparing chat thread and client. Module absence,
ownership mismatch, expiry, and provider errors fail closed; there is no local
quote or signing fallback.

The host controller schemas are in [`schemas.rs`](./schemas.rs). See the
[parent Web3 architecture](../README.md) for the module/host boundary.
