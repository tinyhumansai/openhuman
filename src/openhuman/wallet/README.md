# wallet

Core-owned local multi-chain crypto wallet. Owns onboarding metadata (consent + derived per-chain account addresses), secret material at rest (an encrypted recovery phrase stored in the OS keychain or workspace JSON), and the agent-facing **execution surface**: balance reads, network/asset catalogs, chain readiness, and a prepare-then-confirm-then-execute flow for native sends, token transfers, swaps, and contract calls across EVM (Ethereum + Base/Arbitrum/Optimism/Polygon L2s), Bitcoin (P2WPKH), Solana (native + SPL), and Tron (native + TRC20). Signing and broadcast happen in-core from the decrypted recovery phrase; no private keys ever cross the wire. Swaps are quote-only on every chain.

## Responsibilities

- Persist wallet onboarding state (consent flag, mnemonic word count, setup source, exactly one derived account per supported chain) and the encrypted recovery phrase.
- Prefer the OS keychain for the encrypted mnemonic; transparently migrate the secret out of `wallet-state.json` into the keychain on load/save, and back to JSON when the keychain is unavailable (headless).
- Expose read-only wallet info: status, per-account native balances (EVM live, others provider-gated), supported-asset catalog, per-network defaults (RPC/explorer/capability flags), and per-chain readiness.
- Build prepared-transaction quotes (validated, fee-estimated, TTL'd) that must be explicitly confirmed before execution.
- Sign and broadcast confirmed quotes per chain; restore (and TTL-refresh) the quote on failure so it stays retryable.
- Bind each quote to the chat thread that prepared it so a leaked `quote_id` in a shared channel can't be hijacked from another agent session.
- Expose three agent tools (`wallet_status`, `wallet_chain_status`, `wallet_prepare_transfer`) and eleven `wallet.*` RPC controllers.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/wallet/mod.rs` | Export-focused module root; module docstring, `mod`/`pub use` re-exports. |
| `src/openhuman/wallet/ops.rs` | Onboarding metadata + secret persistence: `WalletChain`/`WalletAccount`/`WalletStatus` types, `setup`/`status`, atomic `wallet-state.json` writes (temp-file + fsync), corrupt-state quarantine, keychain load/save/migrate, `validate_setup`, and `secret_material` (crate-internal) used by chain signers. |
| `src/openhuman/wallet/execution.rs` | Execution surface: balances/network_defaults/supported_assets/chain_status reads, `prepare_transfer`/`prepare_swap`/`prepare_contract_call`/`execute_prepared`, the in-memory quote store (TTL'd, capped at 64), `QuoteOwner` chat-thread binding, amount/address/calldata validation, fee estimation, hex/u256 helpers. |
| `src/openhuman/wallet/defaults.rs` | `EvmNetwork` enum (chain id, default RPC, explorer base, env var), default RPC/REST URLs for BTC/Solana/Tron, env-override resolution, and per-chain/per-network asset catalogs. |
| `src/openhuman/wallet/abi.rs` | `encode_erc20_transfer` — encodes `transfer(address,uint256)` calldata via `ethers_core::abi`. |
| `src/openhuman/wallet/schemas.rs` | RPC controller schemas + `handle_*` dispatchers delegating to `ops`/`execution`; `all_wallet_controller_schemas` / `all_wallet_registered_controllers`. |
| `src/openhuman/wallet/rpc.rs` | **Network transport** (not RPC controllers): shared `reqwest::Client`, JSON-RPC POST (`rpc_call`, `evm_rpc_call`, `rpc_call_to`), REST GET/POST helpers, URL redaction for logs. |
| `src/openhuman/wallet/tools.rs` | Re-exports the three agent tool structs from `tools/`. |
| `src/openhuman/wallet/tools/status.rs` | `WalletStatusTool` (`wallet_status`). |
| `src/openhuman/wallet/tools/chain_status.rs` | `WalletChainStatusTool` (`wallet_chain_status`). |
| `src/openhuman/wallet/tools/prepare_transfer.rs` | `WalletPrepareTransferTool` (`wallet_prepare_transfer`). |
| `src/openhuman/wallet/chains/mod.rs` | Per-chain executor namespace; docstring of the small per-chain surface (`execute_*_quote`, `native_balance`, `validate_*_address`). |
| `src/openhuman/wallet/chains/evm.rs` | EVM key derivation (`ethers_signers` BIP-39), EIP-1559/typed-tx signing, `eth_*` balance/gas/broadcast. |
| `src/openhuman/wallet/chains/btc.rs` | Bitcoin P2WPKH derivation/signing (`bitcoin` crate, secp256k1, BIP-32) + Esplora REST balance/broadcast. |
| `src/openhuman/wallet/chains/solana.rs` | Solana ed25519 (`ed25519_dalek`) derivation, native + SPL transfers, JSON-RPC balance/broadcast. |
| `src/openhuman/wallet/chains/tron.rs` | Tron derivation/signing + TronGrid REST native + TRC20 transfers. |
| `src/openhuman/wallet/test_support.rs` | `#[cfg(test)]` shared plumbing: `TEST_LOCK`, `setup_wallet_in` (deterministic "abandon … about" mnemonic), per-chain sample addresses. |

## Public surface

From `mod.rs` re-exports:
- **Onboarding (`ops`)**: `setup`, `status`, `WalletAccount`, `WalletChain`, `WalletSetupParams`, `WalletSetupSource`, `WalletStatus`; `pub(crate) secret_material`.
- **Execution (`execution`)**: `balances`, `chain_status`, `execute_prepared`, `wallet_network_defaults`, `prepare_contract_call`, `prepare_swap`, `prepare_transfer`, `supported_assets`, `prepared_quotes_for_test`; types `BalanceInfo`, `ChainStatus`, `ExecutePreparedParams`, `ExecutionResult`, `PrepareContractCallParams`, `PrepareSwapParams`, `PrepareTransferParams`, `PreparedKind`, `PreparedStatus`, `PreparedTransaction`, `ProviderStatus`, `SupportedAsset`.
- **Defaults (`defaults`)**: `asset_catalog`, `default_rpc_url`, `env_var_for_chain`, `evm_asset_catalog`, `explorer_tx_url`, `find_asset`, `find_asset_for_network`, `network_defaults`, `rpc_source_for_chain`, `rpc_url_for_chain`, `rpc_url_for_evm_network`, `EvmNetwork`, `RpcSource`, `WalletAssetDefinition`, `WalletNetworkDefaults`.
- **ABI**: `encode_erc20_transfer`.
- **Schemas**: `all_controller_schemas`, `all_registered_controllers`, `all_wallet_controller_schemas`, `all_wallet_registered_controllers`, `schemas`, `wallet_schemas`.

## RPC / controllers

Namespace `wallet` (method form `openhuman.wallet_<function>`), 11 controllers registered via `all_wallet_registered_controllers`:

| Function | Purpose |
| --- | --- |
| `status` | Onboarding status + safe account metadata. |
| `setup` | Persist consent + derived accounts + encrypted mnemonic (all inputs required). |
| `balances` | Native-asset balances per account (EVM live; others provider-gated). |
| `network_defaults` | RPC/explorer/capability flags + asset catalogs per chain. |
| `supported_assets` | Built-in asset catalog incl. default EVM ERC-20s. |
| `encode_erc20_transfer` | Encode `transfer(address,uint256)` calldata (EVM only). |
| `chain_status` | Per-chain readiness + active RPC URL. |
| `prepare_transfer` | Quote a native/token transfer (all four chains). |
| `prepare_swap` | Quote a swap against a caller-supplied router (≤5000 bps slippage). |
| `prepare_contract_call` | Quote an EVM contract call from caller calldata. |
| `execute_prepared` | Confirm (`confirmed: true`) + execute a quote by `quoteId`. |

Wired into the registry in `src/core/all.rs` (controllers + schemas + capability description).

## Agent tools

Owned in `tools/`, re-exported via `tools.rs`:
- `WalletStatusTool` — `wallet_status`
- `WalletChainStatusTool` — `wallet_chain_status`
- `WalletPrepareTransferTool` — `wallet_prepare_transfer`

All implement `crate::openhuman::tools::traits::Tool` and delegate to the matching `wallet::*` functions. (There is no agent tool for `execute_prepared` here; execution is reached via RPC.)

## Events

None. The module publishes/subscribes no `DomainEvent`s and has no `bus.rs`. Chat-context coupling is via the task-local `approval::APPROVAL_CHAT_CONTEXT`, not the event bus.

## Persistence

- **`{workspace_dir}/state/wallet-state.json`** — `StoredWalletState`: consent flag, source, mnemonic word count, accounts, `updated_at_ms`, and (only as fallback) the encrypted mnemonic. Written atomically (temp file + `sync_all` + dir fsync + `persist`), guarded by a process-wide `WALLET_STATE_FILE_LOCK`. Corrupt/unreadable/invalid files are quarantined to `…json.corrupted.<ts>`.
- **OS keychain** — preferred home for the encrypted mnemonic under key `wallet.mnemonic`, scoped by a workspace-derived user id (`crate::openhuman::keyring`). When available, the secret is stripped from JSON; load promotes any JSON-resident secret into the keychain.
- **In-memory quote store** (`execution.rs`) — `PreparedTransaction`s, 5-minute TTL, cap 64, pruned on access. Not persisted across restarts.

## Dependencies

- `crate::openhuman::config` (`Config`, `config::rpc::load_config_with_timeout`) — resolves workspace dir and config for state paths, keychain user id, and decryption.
- `crate::openhuman::keyring` (`is_available`/`get`/`set`) — OS keychain storage for the encrypted mnemonic.
- `crate::openhuman::encryption::rpc` (`encrypt_secret`/`decrypt_secret`) — chain signers decrypt the recovery phrase before derivation.
- `crate::openhuman::approval::APPROVAL_CHAT_CONTEXT` — task-local chat owner (`thread_id`/`client_id`) used to bind quotes to their originating thread.
- `crate::openhuman::tools::traits` — `Tool`/`ToolResult`/`ToolCallOptions` for the agent tools.
- `crate::core::all` (`ControllerFuture`, `RegisteredController`) and `crate::core` (`ControllerSchema`, `FieldSchema`, `TypeSchema`) — RPC controller registry wiring.
- `crate::rpc::RpcOutcome` — standard RPC return shape.
- External crates: `ethers_core` / `ethers_signers` / `coins_bip39` (EVM + ABI + BIP-39), `bitcoin` + `secp256k1` (BTC), `ed25519_dalek` (Solana), `sha2`, `hex`, `reqwest`, `serde`/`serde_json`, `tempfile`, `parking_lot`, `once_cell`.

## Used by

- `src/openhuman/tools/mod.rs` & `src/openhuman/tools/ops.rs` — register the three wallet agent tools.
- `src/openhuman/tools/impl/network/polymarket.rs` — uses `secret_material`, `status`, and `WalletChain` to derive an EVM signer for Polymarket CLOB.
- `src/openhuman/agent/agents/loader.rs` — references the wallet tools when assembling agent toolsets.
- `src/core/all.rs` — wires controllers/schemas/capability description.
- `src/openhuman/test_support/introspect.rs` — introspection in tests.

## Notes / gotchas

- **`rpc.rs` here is network transport, not RPC controllers.** RPC controllers live in `schemas.rs`. This is an exception to the canonical "`rpc.rs` = domain API" convention.
- **Quote-owner binding** (`execution.rs`): `execute_prepared` only runs when the caller's `current_owner()` equals the prepare-time owner. On mismatch it returns the byte-identical `quote '…' not found` error as a true miss — no enumeration oracle. Non-chat callers (CLI / direct RPC / background/cron) have `owner == None` and can only execute quotes they also prepared with no chat context. `current_owner()` relies on the inline `.await` chain in `channels/providers/web.rs::run_chat_task`; detaching the tool loop onto a fresh `tokio::spawn` without re-scoping `APPROVAL_CHAT_CONTEXT` would silently disable the gate.
- **Quotes are consumed atomically**: `take_quote_for` removes the quote before broadcast so concurrent confirmations can't double-submit; on failure the quote is restored with a refreshed TTL.
- **Setup requires exactly one account per chain** (EVM, BTC, Solana, Tron) and a non-empty encrypted mnemonic; valid mnemonic word counts are 12/15/18/21/24.
- **EVM is one `WalletChain::Evm` variant across 5 networks** selected by `EvmNetwork` (defaults to `ethereum_mainnet`); other chains ignore `evmNetwork`. BTC rejects token transfers; contract calls are EVM-only; swaps are quote-only everywhere.
- **RPC endpoints are overridable** per chain/network via `OPENHUMAN_WALLET_RPC_*` env vars (used by tests pointing at an axum mock). Log lines redact URLs to scheme+host.
- **`balances`**: only EVM reads live (Ethereum mainnet); BTC/Solana/Tron call their providers but fall back to zero with `ProviderStatus::Missing` on error.
