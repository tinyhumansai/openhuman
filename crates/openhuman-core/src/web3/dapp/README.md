# web3_dapp

Generic EVM contract calls accept a target contract and pre-encoded calldata.
The compiled TinyWallet module validates the request, prepares and stores the
quote, and later signs and broadcasts after explicit approval. OpenHuman keeps
the `web3_dapp` RPC schemas and the typed module adapter.

## Operations

| Function | Purpose |
| --- | --- |
| `call` | Prepare an EVM contract call with `contractAddress`, `calldata`, and optional `valueRaw` / `evmNetwork`. |
| `execute` | Execute a stored quote with `confirmed: true`; the host approves the exact transaction intent before module signing. |

No backend call is needed to prepare a dapp call. Quotes are module-owned and
bound to the preparing chat thread and client. The host does not validate or
encode chain transactions locally.

The host controller schemas are in [`schemas.rs`](./schemas.rs). See the
[parent Web3 architecture](../README.md) for the module/host boundary.
