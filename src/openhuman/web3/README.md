# web3

High-level web3 surface built **on top of** the [`wallet`](../wallet/README.md)
module. The wallet stays basic (keys, balances, transfers, tx inspection); this
module focuses on EVM/Solana(/BTC) dapp interactions: **swaps**, **bridges**, and
generic **dapp contract calls**.

Quotes and ready-to-sign **unsigned transactions** come from the openhuman
backend's deBridge proxy (`/agent-integrations/crypto/{routes,swap,bridge}`,
backend PR #852). This module only resolves the caller's wallet address,
forwards the request, and stores a confirm→execute quote. On execute it hands
the unsigned transaction to the wallet's crate-internal signing primitives —
`wallet::sign_and_broadcast_evm` (EVM `to`/`data`/`value`) or
`wallet::sign_and_broadcast_solana` (a hex `VersionedTransaction`) — so private
keys never leave the wallet.

## Three sub-modules (three RPC namespaces + tool families)

| Module | Namespace | Purpose |
| --- | --- | --- |
| `swap/` | `web3_swap` | Single-chain swaps via deBridge. Cross-chain requests are rejected with a pointer to `web3_bridge`. |
| `bridge/` | `web3_bridge` | Cross-chain bridges via deBridge DLN. Same-chain requests rejected. Signs+broadcasts on the **source** chain. |
| `dapp/` | `web3_dapp` | Generic EVM contract calls from caller-supplied calldata (no backend). |

## Key files

| File | Role |
| --- | --- |
| `mod.rs` | Export-focused root: aggregates `all_web3_controller_schemas` / `all_web3_registered_controllers` / `all_web3_agent_tools`, plus shared schema/tool helpers. |
| `types.rs` | deBridge chain-id ↔ local signer mapping (`chain_family`, `DEBRIDGE_SOLANA_CHAIN_ID`), request params, `UnsignedTx`, `Web3QuoteKind`. |
| `client.rs` | `CryptoClient` — thin wrapper over the shared `IntegrationClient` for `/agent-integrations/crypto/*` (Bearer JWT auth, envelope unwrap). |
| `store.rs` | In-memory prepared-quote store (TTL'd, capped, chat-thread owner-bound like the wallet) + the shared confirm→execute path. |
| `ops.rs` | Shared op logic: `routes`, `quote_swap`, `quote_bridge`, `prepare_dapp_call` (address defaulting, backend call, unsigned-tx extraction). |
| `{swap,bridge,dapp}/schemas.rs` | Per-namespace RPC controllers + handlers. |
| `{swap,bridge,dapp}/tools.rs` | Per-namespace agent tools. |

## RPC / controllers

- `web3_swap`: `quote`, `execute`, `routes` (`openhuman.web3_swap_quote`, …).
- `web3_bridge`: `quote`, `execute`.
- `web3_dapp`: `call`, `execute`.

Quote/call methods return a `quoteId`; the matching `*_execute` (with
`confirmed: true`) signs and broadcasts. Quotes are bound to the chat thread
that prepared them (no cross-session hijack of a leaked `quoteId`), TTL'd at
5 minutes, and restored with a refreshed TTL on broadcast failure.

## Agent tools

`web3_swap_quote`, `web3_swap_execute`, `web3_swap_routes`,
`web3_bridge_quote`, `web3_bridge_execute`, `web3_dapp_call`,
`web3_dapp_execute` (registered in `src/openhuman/tools/ops.rs`). They call the
backend per-invocation and error gracefully when the user is not signed in.

## Chain-id mapping

deBridge uses real EVM chain ids (1 ETH, 10 Optimism, 56 BNB, 137 Polygon,
8453 Base, 42161 Arbitrum) and a synthetic Solana id (`7565164`). `chain_family`
maps a deBridge id to the local signer family; ids we can't sign for are
rejected at quote time.

## Dependencies

- [`crate::openhuman::wallet`] — `sign_and_broadcast_evm` / `sign_and_broadcast_solana` (crate-internal), `status` for address resolution, `EvmNetwork` / `WalletChain`.
- [`crate::openhuman::integrations`] (`IntegrationClient`, `build_client`) — backend auth + transport.
- `crate::openhuman::approval::APPROVAL_CHAT_CONTEXT` — quote-owner binding.
- `crate::core::all` / `crate::core` — RPC controller registry wiring.

## Notes / gotchas

- The backend injects a configurable affiliate fee (default 1%) collected on-chain by deBridge on the source chain; this module passes the quote through unchanged.
- deBridge returns a large nested object; everything not explicitly consumed is passed through as `serde_json::Value`.
- Same-chain `web3_bridge` requests and cross-chain `web3_swap` requests are rejected (mirrors deBridge's single-chain swap vs. cross-chain DLN split).
