# wallet

Core-owned local multi-chain crypto wallet, deliberately basic: key/account
management plus the primitive on-chain operations. Owns onboarding metadata
(consent + derived per-chain account addresses), secret material at rest (an
encrypted recovery phrase stored in the OS keychain or workspace JSON), and
the agent-facing surface: address and balance reads, network/asset catalogs,
chain readiness, a prepare-then-confirm-then-execute flow for native sends and
token transfers (ERC20 / SPL / TRC20 / BEP20), transaction broadcast, and
read-only transaction inspection (status, receipt, lookup) across EVM
(Ethereum plus Base/Arbitrum/Optimism/Polygon/BNB Chain), Bitcoin (P2WPKH),
Solana (native + SPL), and Tron (native + TRC20). This process decrypts the
recovery phrase and hands it, as a `tinywallet_bus::wire::SecretMaterial`, to
the loaded `tinywallet` native module (`crate::modules::wallet`), which
derives the key, signs, and returns the raw signed bytes; the core then
broadcasts. The binary never assembles a private key outside `cfg(test)`.

Higher-level DeFi affordances (swaps, bridges, generic dapp/contract calls)
live in the separate [`web3`](../README.md) module, which builds on the
engine's `sign_and_broadcast_evm` / `sign_and_broadcast_solana` primitives.
They are not part of the wallet's agent or RPC surface.

## Where the logic lives

The wallet engine (balances, the prepare/confirm/execute flow, the per-chain
signing choreography, static reference data, the agent tools) lives in the
vendored `tinywallet-web3` crate (`vendor/tinywallet/crates/tinywallet-web3`,
`crypto::{wallet, execution, chains, defaults, abi}` and `tools::wallet`). What
stays here is the host: onboarding state and secrets ([`ops`](./ops)), the `Outcome`
adapters ([`execution.rs`](./execution.rs)), endpoint resolution from the environment
([`endpoints.rs`](./endpoints.rs)), the host `Transport` ([`transport.rs`](./transport.rs), [`rpc.rs`](./rpc.rs)) and the RPC
controllers ([`schemas.rs`](./schemas.rs)). The crate reaches this module only through seams,
implemented in [`web3/seams.rs`](../seams.rs).

## Compile-time gate (`web3` feature)

`pub mod wallet;` in `web3/mod.rs` is always compiled: it is a facade. The
real implementation ([`ops`](./ops), `execution`, `endpoints`, `schemas`, `tools`,
`transport`) is gated behind the default-ON `web3` Cargo
feature (shared with `web3` and `web3::x402`). When the feature is off,
`stub` takes its place and mirrors the subset of the public surface that
always-on or other-gated callers depend on: `WALLET_NOT_CONFIGURED_MESSAGE`,
`status`, `secret_material`, `WalletChain`, `prepare_transfer`,
`execute_prepared`, the prepare/execute param and result types,
`solana_cluster` / `SolanaCluster`, `prepared_quotes_for_test`, and the
controller-registration entry points (`all_wallet_registered_controllers`,
`all_wallet_controller_schemas`), with no-op or disabled-error bodies.
Signatures must match the real ones exactly; `cargo check
--no-default-features` is the only thing that catches drift.

## Responsibilities

- Persist wallet onboarding state (consent flag, mnemonic word count, setup source, exactly one derived account per supported chain) and the encrypted recovery phrase.
- Prefer the OS keychain for the encrypted mnemonic; transparently migrate the secret out of `wallet-state.json` into the keychain on load/save, and back to JSON when the keychain is unavailable (headless).
- Expose read-only wallet info: status, per-account native balances (EVM live, others provider-gated), supported-asset catalog, per-network defaults (RPC/explorer/capability flags), and per-chain readiness.
- Build prepared-transaction quotes (validated, fee-estimated, TTL'd) that must be explicitly confirmed before execution.
- Sign and broadcast confirmed quotes per chain; restore (and TTL-refresh) the quote on failure so it stays retryable.
- Bind each quote to the chat thread that prepared it so a leaked `quote_id` in a shared channel can't be hijacked from another agent session.
- Expose six agent tools (`wallet_status`, `wallet_chain_status`, `wallet_prepare_transfer`, `wallet_tx_status`, `wallet_tx_receipt`, `wallet_lookup_tx`) and thirteen `wallet.*` RPC controllers.
- The engine provides `sign_and_broadcast_evm` / `sign_and_broadcast_solana` primitives for the `web3` layer (sign and broadcast an externally-built unsigned transaction). Not exposed to the agent or RPC surface.

## Key files

| File | Role |
| --- | --- |
| `crates/openhuman-core/src/web3/wallet/mod.rs` | Export-focused module root; module docstring, `mod`/`pub use` re-exports. |
| `crates/openhuman-core/src/web3/wallet/ops.rs` | Onboarding metadata and secret persistence: `setup`/`status`/`reveal_recovery_phrase`, atomic `wallet-state.json` writes (temp-file + fsync), corrupt-state quarantine, keychain load/save/migrate, `validate_setup`, and `secret_material` (crate-internal) used by the host signer. [`ops/types.rs`](./ops/types.rs) re-exports `WalletChain`/`WalletAccount`/`WalletStatus`/`WALLET_NOT_CONFIGURED_MESSAGE` from `tinywallet-web3` and keeps the persisted shape and the setup params. |
| `crates/openhuman-core/src/web3/wallet/execution.rs` | `Outcome` adapters over the process-wide engine: `balances`/`network_defaults`/`supported_assets`/`chain_status`, `prepare_transfer`/`execute_prepared`, `tx_status`/`tx_receipt`/`lookup_tx`, `prepared_quotes_for_test`. Re-exports the engine's wire types. |
| `crates/openhuman-core/src/web3/wallet/endpoints.rs` | The `OPENHUMAN_WALLET_RPC_*` / `OPENHUMAN_SOLANA_CLUSTER` environment resolution, and `HostEndpoints`, the `RpcEndpoints` seam. The static defaults are in the crate. |
| `crates/openhuman-core/src/web3/wallet/schemas.rs` | RPC controller schemas and `handle_*` dispatchers delegating to `ops`/`execution`; `all_wallet_controller_schemas` / `all_wallet_registered_controllers`. |
| `crates/openhuman-core/src/web3/wallet/rpc.rs` | Network transport, not RPC controllers: shared `reqwest::Client`, JSON-RPC POST (`rpc_call_to`), REST GET/POST helpers, URL redaction for logs. |
| `crates/openhuman-core/src/web3/wallet/transport.rs` | OpenHuman's implementation of the `tinywallet_bus::rpc::Transport` seam: resolves a `tinywallet_bus::rpc::NetworkId` to an endpoint (including `OPENHUMAN_WALLET_RPC_<CHAIN>` overrides), redacts URLs for logs, and reuses [`rpc.rs`](./rpc.rs)'s shared `reqwest` client. Classifies errors conservatively: anything it cannot prove is a transport failure is reported as `TransportError::Rpc` (authoritative) rather than `Unreachable` (retryable), so an unclassifiable error stops a failover loop instead of risking a double broadcast. |
| `crates/openhuman-core/src/web3/wallet/tools.rs` | Re-exports the six agent tool structs from `tinywallet_web3::tools::wallet`. |
| `crates/openhuman-core/src/web3/wallet/stub.rs` | Disabled-wallet facade compiled when `web3` is off; mirrors the subset of the real surface that always-on or other-gated callers need, with no-op or disabled-error bodies. See the Compile-time gate section. |
| `crates/openhuman-core/src/web3/wallet/test_support.rs` | `#[cfg(test)]` shared plumbing for the keyring/setup tests: `TEST_LOCK`, `setup_wallet_in` (deterministic "abandon ... about" mnemonic), per-chain sample addresses. The chain, execution and quote tests moved to `tinywallet-web3` with fakes. |

## Public surface

From [`mod.rs`](./mod.rs) re-exports:

- Onboarding (`ops`): `setup`, `status`, `reveal_recovery_phrase`, `RevealRecoveryPhraseResult`, `WalletAccount`, `WalletChain`, `WalletSetupParams`, `WalletSetupSource`, `WalletStatus`, `WALLET_NOT_CONFIGURED_MESSAGE`; `pub(crate) secret_material`.
- Execution (`execution`): `balances`, `chain_status`, `execute_prepared`, `wallet_network_defaults`, `prepare_transfer`, `tx_status`, `tx_receipt`, `lookup_tx`, `supported_assets`, `prepared_quotes_for_test`; types `BalanceInfo`, `ChainStatus`, `ExecutePreparedParams`, `ExecutionResult`, `PrepareTransferParams`, `PreparedKind`, `PreparedStatus`, `PreparedTransaction`, `ProviderStatus`, `SupportedAsset`, `TxState`, `TxStatusInfo`, `TxReceiptInfo`, `TxLookupInfo` (all from `tinywallet-web3`).
- Defaults: `solana_cluster` (host, env-driven) and, from `tinywallet-web3`, `evm_asset_catalog`, `explorer_tx_url`, `EvmNetwork`, `RpcSource`, `SolanaCluster`, `WalletAssetDefinition`, `WalletNetworkDefaults`.
- ABI: `encode_erc20_transfer` (from `tinywallet-web3`).
- Schemas: `all_controller_schemas`, `all_registered_controllers`, `all_wallet_controller_schemas`, `all_wallet_registered_controllers`, `schemas`, `wallet_schemas`.

## RPC / controllers

Namespace `wallet` (method form `openhuman.wallet_<function>`), 13 controllers registered via `all_wallet_registered_controllers`:

| Function | Purpose |
| --- | --- |
| `status` | Onboarding status + safe account metadata (addresses). |
| `setup` | Persist consent + derived accounts + encrypted mnemonic (all inputs required); refuses to overwrite a configured wallet unless `force: true`. |
| `balances` | Native-asset balances per account (EVM live; others provider-gated). |
| `network_defaults` | RPC/explorer/capability flags + asset catalogs per chain. |
| `supported_assets` | Built-in asset catalog including default EVM ERC-20s / BEP20s. |
| `encode_erc20_transfer` | Encode `transfer(address,uint256)` calldata (EVM only). |
| `chain_status` | Per-chain readiness + active RPC URL. An account's endpoint is probed (chain tip) through the transport; a failure reports `missing` with `error`. |
| `prepare_transfer` | Quote a native/token transfer (all four chains). |
| `execute_prepared` | Confirm (`confirmed: true`) and execute a quote by `quoteId` (tx send). |
| `tx_status` | Check a transaction's lifecycle state (pending/confirmed/failed/not_found) by hash. |
| `tx_receipt` | Fetch a transaction receipt (success, fee, block) by hash. |
| `lookup_tx` | Look up the raw transaction payload by hash. |
| `reveal_recovery_phrase` | Decrypt and return the stored BIP-39 phrase (`{phrase, wordCount}`); read-only, for transient display in the UI. |

Wired into the registry in `crates/openhuman-core/src/core/all.rs` (controllers + schemas + capability description).

## Agent tools

Defined in `tinywallet_web3::tools::wallet`, re-exported via [`tools.rs`](./tools.rs), built over the process-wide engine in `tools/ops.rs`:

- `WalletStatusTool`, tool name `wallet_status`
- `WalletChainStatusTool`, tool name `wallet_chain_status`
- `WalletPrepareTransferTool`, tool name `wallet_prepare_transfer`
- `WalletTxStatusTool`, tool name `wallet_tx_status`
- `WalletTxReceiptTool`, tool name `wallet_tx_receipt`
- `WalletLookupTxTool`, tool name `wallet_lookup_tx`

All implement `tinytools::Tool` and delegate to the matching `wallet::*` functions. There is no agent tool for `execute_prepared` here; execution is reached via RPC.

## Events

None. The module publishes or subscribes no `DomainEvent`s and has no `bus.rs`. Chat-context coupling is via the task-local `approval::APPROVAL_CHAT_CONTEXT`, not the event bus.

## Persistence

- `{workspace_dir}/state/wallet-state.json`: `StoredWalletState`, consent flag, source, mnemonic word count, accounts, `updated_at_ms`, and (only as fallback) the encrypted mnemonic. Written atomically (temp file + `sync_all` + dir fsync + `persist`), guarded by a process-wide `WALLET_STATE_FILE_LOCK`. Corrupt/unreadable/invalid files are quarantined to `...json.corrupted.<ts>`.
- OS keychain: preferred home for the encrypted mnemonic under key `wallet.mnemonic`, scoped by a workspace-derived user id (`crate::security::secret_storage`), used only when `keyring_consent::policy::check_secret_access()` returns `Proceed` and the keyring is available. When available, the secret is stripped from JSON; load promotes any JSON-resident secret into the keychain.
- In-memory quote store (instance-owned by the `WalletEngine`, `tinywallet_web3::quote::QuoteStore`): `PreparedTransaction`s, 5-minute TTL, cap 64, pruned on insert. Not persisted across restarts.

## Dependencies

- `crate::config` (`Config`, `config::rpc::load_config_with_timeout`): resolves workspace dir and config for state paths, keychain user id, and decryption.
- `crate::security::secret_storage` (`is_available`/`get`/`set`): OS keychain storage for the encrypted mnemonic.
- `crate::security::encryption::rpc` (`encrypt_secret`/`decrypt_secret`): `HostSigner` decrypts the recovery phrase before handing it to the wallet module.
- `crate::modules::wallet` (`derive_account`, `sign_transaction_in_module`, `sign_message`): the loaded `tinywallet` native module does every derivation and signature; the phrase is only sent after the module passes the attestation check (`modules::wallet::attested_proxy`).
- `crate::security::approval::APPROVAL_CHAT_CONTEXT`: task-local chat owner (`thread_id`/`client_id`), read by `TaskLocalScope`, used to bind quotes to their originating thread.
- `tinywallet-web3` (optional, feature `tools`, enabled by `web3`): the engine, chains, quote store and agent tools; the host implements its seams in `web3/seams.rs`.
- `crate::core::all` (`ControllerFuture`, `RegisteredController`) and `crate::core` (`ControllerSchema`, `FieldSchema`, `TypeSchema`): RPC controller registry wiring.
- `crate::core::Outcome`: standard RPC return shape.
- `tinywallet-bus` (`vendor/tinywallet/crates/tinywallet-bus`, optional, gated by the `web3` feature; features `btc`, `evm`, `solana`, `tron`, `keccak`, `net`, `wire`, `eip712`, `abi`, `tx-codec`): the contract crate. It owns address formats (parsing/validation/conversion) and the wire types crossing the `Transport` seam (`SecretMaterial`, `TransactionSpec`, `NetworkId`), the ERC-20/EIP-712 encoders, and the Tron verifier. Per Cargo.toml's own rationale, this is taken as the contract crate and not the root `tinywallet` crate: key derivation, transaction building, signing and the chain clients (including the `bitcoin` crate and its native secp256k1 build) live inside that loaded module now, so this binary links none of it. The root `tinywallet` crate is still a dev-dependency, used only so test fixtures can derive a known account from a BIP-39 vector phrase.
- Other external crates: `reqwest`, `serde`/`serde_json`, `tempfile`, `parking_lot`, `once_cell`. `curve25519-dalek`, `bs58`, `sha2` and the Solana wire code moved to `tinywallet-web3`; `k256`, `coins-bip39` and the root `tinywallet` crate remain test-only dev-dependencies for the keyring/setup fixtures.

## Used by

- `registration::registered_tools` constructs the six wallet agent tools from `tinywallet-web3`; `crates/openhuman-core/src/tools/ops.rs` adds that domain factory (gated on the `web3` feature) and reserves the `wallet_`/`web3_`/`x402_` name prefixes as Web3-exclusive.
- `crates/openhuman-core/src/core/all.rs` wires controllers/schemas/capability description.
- `crates/openhuman-core/src/test_support/introspect.rs`: `wallet_prepared_quotes` introspection helper used in tests, backed by `wallet::prepared_quotes_for_test`.

## Notes / gotchas

- `rpc.rs` here is network transport, not RPC controllers. RPC controllers live in [`schemas.rs`](./schemas.rs). This is an exception to the canonical "`rpc.rs` = domain API" convention.
- Quote-owner binding (`tinywallet_web3::quote::QuoteStore`): `execute_prepared` only runs when the caller's `QuoteScope::current_owner()` equals the prepare-time owner. On mismatch it returns the byte-identical `quote '...' not found` error as a true miss, not an enumeration oracle. Non-chat callers (CLI, direct RPC, background/cron) have `owner == None` and can only execute quotes they also prepared with no chat context. The host's `TaskLocalScope` (`web3/seams.rs`) reads `APPROVAL_CHAT_CONTEXT` synchronously on the tool's own task and relies on the inline `.await` chain in `web_chat::run_chat_task`; detaching the tool loop onto a fresh `tokio::spawn` without re-scoping `APPROVAL_CHAT_CONTEXT` would silently disable the gate.
- Quotes are consumed atomically: `QuoteStore::take_for` removes the quote before broadcast so concurrent confirmations can't double-submit; on failure the quote is restored with a refreshed TTL.
- Setup requires exactly one account per chain (EVM, BTC, Solana, Tron) and a non-empty encrypted mnemonic; valid mnemonic word counts are 12/15/18/21/24.
- EVM is one `WalletChain::Evm` variant across 6 networks (Ethereum, Base, Arbitrum, Optimism, Polygon, BNB Chain) selected by `EvmNetwork` (defaults to `ethereum_mainnet`); other chains ignore `evmNetwork`. BTC rejects token transfers. Swaps, bridges and contract calls are not in the wallet; they live in the [`web3`](../README.md) module.
- RPC endpoints are overridable per chain/network via `OPENHUMAN_WALLET_RPC_*` env vars (used by tests pointing at an axum mock). Log lines redact URLs to scheme and host.
- `balances`: only EVM reads live (Ethereum mainnet); BTC/Solana/Tron call their providers but fall back to zero with `ProviderStatus::Missing` on error.

## Further reading

- [Parent module (`web3`)](../README.md)
- [Wallet](../../../../../gitbooks/features/wallet.md)
- [tinywallet submodule](../../../../../vendor/tinywallet/README.md)
- [Loadable modules](../../../../../gitbooks/developing/loadable-modules.md)
