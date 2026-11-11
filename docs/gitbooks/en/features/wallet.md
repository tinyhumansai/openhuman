---
description: >-
  A local, non-custodial multi-chain crypto wallet the agent can read balances
  from and prepare transfers with. Keys stay in the core and never cross the
  wire.
icon: wallet
---

# Wallet

The wallet is a basic, non-custodial multi-chain crypto wallet owned by the Rust core. It keeps one account per supported chain, all derived from a single recovery phrase. It reads balances and runs a strict prepare, confirm, execute flow for native sends and a small set of token transfers.

It is minimal on purpose: key and account management, plus the primitive on-chain operations. Higher-level DeFi (swaps, bridges, generic contract and dapp calls) lives in a separate `web3` module and is not part of the wallet's agent or RPC surface.

The key point: signing and broadcast happen entirely in the core, from the decrypted recovery phrase. No private key ever leaves the device or crosses the network. This is your money, so the wallet is conservative by design.

## Supported chains and token standards

Setup derives exactly one account per chain. The EVM account is reused across six networks. Only the standards below are supported for transfers. Anything else, such as swaps or arbitrary contract calls, is out of scope.

| Chain | Networks | Native | Token standard | Notes |
| --- | --- | --- | --- | --- |
| EVM | Ethereum, Base, Arbitrum, Optimism, Polygon, BNB Chain | ETH, BNB and others | ERC-20 (BEP-20 on BNB Chain) | One `Evm` account across all six. The network is chosen per request and defaults to Ethereum mainnet. EIP-1559 and typed-tx signing. |
| Bitcoin | Mainnet | BTC | None | P2WPKH (native SegWit). Rejects token transfers. Esplora REST for balance and broadcast. |
| Solana | Mainnet or devnet (per RPC) | SOL | SPL | ed25519 signing. Native and SPL token transfers. |
| Tron | Mainnet | TRX | TRC-20 | TronGrid REST for native and TRC-20 transfers. |

The built-in asset catalogs include the native asset plus common stablecoins per chain, for example USDC and USDT as ERC-20 or BEP-20, USDC as SPL on Solana, and USDT as TRC-20 on Tron.

## Onboarding and the recovery phrase

Setup is a single, all-or-nothing operation. It saves a consent flag, the mnemonic word count, the setup source, exactly one derived account per supported chain, and the encrypted recovery phrase. Valid BIP-39 word counts are 12, 15, 18, 21 and 24.

The recovery phrase is the only secret. The wallet stores the per-chain account addresses (safe to show) apart from the secret material. Only the encrypted phrase can rebuild private keys.

## Key custody and security

The wallet is non-custodial and local. There is no server-side key escrow.

- The recovery phrase is always encrypted at rest. The core `encryption` domain encrypts it before it is saved anywhere.
- The preferred home is the OS keychain. The encrypted phrase lives there under the key `wallet.mnemonic`, scoped by a workspace-derived user id. The keyring consent policy gates access.
- When the keychain is unavailable (for example on a headless machine), the encrypted phrase falls back to `{workspace_dir}/state/wallet-state.json`. When a keychain becomes available, any secret in that JSON is migrated into it and removed from the JSON.
- Writes to `wallet-state.json` are atomic (temp file, fsync, persist) under a process-wide lock. Corrupt or invalid state files are quarantined, not trusted.
- Chain signers decrypt the phrase in the core only when deriving a key to sign a confirmed transaction. Plaintext keys are never saved and never sent over the wire.

See [OS keyring and secret storage](os-keyring-and-secret-storage.md) for how secrets are stored on each platform, and [Privacy and security](privacy-and-security.md) for the broader model.

## Reading balances and chain info

Read-only surfaces need no confirmation:

- **Status:** onboarding state plus the safe per-chain account addresses.
- **Balances:** native-asset balances per account. Only EVM balances read live today (Ethereum mainnet). BTC, Solana and Tron call their providers but fall back to a zero balance with a "provider missing" status on error.
- **Network defaults and supported assets:** per-chain RPC and explorer URLs, capability flags and the built-in asset catalog.
- **Chain status:** per-chain readiness and the active RPC URL.

You can override RPC endpoints per chain and network with `OPENHUMAN_WALLET_RPC_*` environment variables. Logs redact URLs to scheme and host.

## Sending transfers: prepare, confirm, execute

Every write is a two-step, deliberate flow. The wallet never sends in one shot.

1. **Prepare** (`prepare_transfer`) validates the amount, the destination address and (for tokens) the calldata, estimates fees, and returns a prepared quote with a `quoteId`. Quotes sit in an in-memory store with a 5-minute TTL, capped at 64. They are not kept across restarts.
2. **Confirm and execute** (`execute_prepared`) needs `confirmed: true` and a valid `quoteId`. The quote is consumed atomically before broadcast, so concurrent confirmations cannot double-submit. On failure it is restored with a refreshed TTL, so you can retry.

Each quote is bound to the chat thread that prepared it. Only that owner can execute it. A `quoteId` leaked into a shared channel returns the same "not found" error as an unknown id, so another session cannot hijack it.

Transfers are limited to native sends and the token standards in the table above. Bitcoin rejects token transfers. Swaps, bridges and generic contract calls are not available here.

## Transaction status tracking

After broadcast, three read-only tools let the agent follow a transaction by hash:

- `tx_status`: lifecycle state (pending, confirmed, failed or not found).
- `tx_receipt`: receipt details (success, fee, block).
- `lookup_tx`: the raw transaction payload.

## Agent tools and approval safety

The agent reaches the wallet through eight tools:

| Tool | Purpose |
| --- | --- |
| `wallet_status` | Onboarding status and account addresses. |
| `wallet_chain_status` | Per-chain readiness and active RPC. |
| `wallet_balances` | Live native and token (USDC on Solana) balances, as `{native, tokens}`. If the token read fails, `tokens` is empty and `tokensError` says why; the native rows are still returned. |
| `wallet_prepare_transfer` | Build a validated, fee-estimated quote. |
| `wallet_execute_prepared` | Execute a prepared quote, only within the spend policy. |
| `wallet_tx_status` | Transaction lifecycle state by hash. |
| `wallet_tx_receipt` | Transaction receipt by hash. |
| `wallet_lookup_tx` | Raw transaction lookup by hash. |

The wallet UI's own execute is unchanged: you press confirm, and it is not capped unless you ask for that. The agent's execute tool is always held to the **spend policy**, and with no policy configured it cannot move funds at all, so you finish the transfer in the wallet UI as before.

A quote is bound to the conversation that asked for it, expires after five minutes and is consumed when it executes. The [approval gate](approval-gate.md) is a further layer for chat turns. It applies only once you turn on the autonomy policy, which is off by default. Treat every transfer as high-stakes and read that page before relying on the gate.

## Spend policy for automated payments

The `[wallet_policy]` section of `config.toml` lets an agent pay without you confirming each transfer, within limits you set. Background jobs and workflows that do not require approval may execute only when `automation_enabled` is on. Every asset must be listed, with optional caps in atomic units (lamports for SOL, millionths for USDC):

- `per_tx_max_raw`: the largest single transfer.
- `daily_max_raw`: the total over the last 24 hours (rolling).
- `recipient_weekly_max_raw`: the total to one address over the last 7 days (rolling).

There is no price feed, so a dollar cap is written as the amount it corresponds to. Fees are not counted. Example, SOL and USDC only, $50 per transfer, $100 per day, $50 per recipient per week (SOL at $150):

```toml
[wallet_policy]
automation_enabled = true

[[wallet_policy.assets]]
chain = "solana"
symbol = "USDC"
per_tx_max_raw = 50000000
daily_max_raw = 100000000
recipient_weekly_max_raw = 50000000

[[wallet_policy.assets]]
chain = "solana"
symbol = "SOL"
per_tx_max_raw = 333333333
daily_max_raw = 666666666
recipient_weekly_max_raw = 333333333
```

The caps of one asset are counted separately from the other's. `OPENHUMAN_WALLET_POLICY_AUTOMATION_ENABLED`, `OPENHUMAN_WALLET_POLICY_APPLY_TO_INTERACTIVE` and `OPENHUMAN_WALLET_POLICY_ASSETS` (a JSON array of the same asset entries) override the file. The `wallet.policy_get`, `wallet.policy_update` and `wallet.ledger_list` RPCs read, change and audit it.

Every execute is written to a ledger before it is signed and updated afterwards. A failed execute is marked *in doubt*: it still counts against the caps and is never retried, so check its status before preparing a new transfer. Scheduled (cron) agent turns are read-only and cannot pay. x402 payments from automation are limited by the x402 budget instead.

## See also

- [Approval gate](approval-gate.md)
- [Privacy and security](privacy-and-security.md)
- [OS keyring and secret storage](os-keyring-and-secret-storage.md)
