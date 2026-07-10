---
description: >-
  Local, non-custodial multi-chain crypto wallet the agent can read balances
  from and send transfers with. Keys stay in-core and never cross the wire.
icon: wallet
---

# Wallet

A deliberately **basic**, **non-custodial** multi-chain crypto wallet owned by the Rust core. It manages one account per supported chain derived from a single recovery phrase, reads balances, and runs a strict **prepare → confirm → execute** flow for native sends and a small set of token transfers.

It is intentionally minimal: key/account management plus the primitive on-chain operations. Higher-level DeFi (swaps, bridges, generic contract/dapp calls) lives in a separate `web3` module and is **not** part of the wallet's agent or RPC surface.

The most important property to understand: **signing and broadcast happen entirely in-core from the decrypted recovery phrase. No private keys ever leave the device or cross the network.** This is your money, so the wallet is conservative by design.

***

## Supported chains and token standards

Setup derives **exactly one account per chain**. EVM is a single account reused across six networks. Only the standards listed below are supported for transfers. Anything else (swaps, arbitrary contract calls) is out of scope for the wallet.

| Chain | Networks | Native | Token standard | Notes |
| --- | --- | --- | --- | --- |
| EVM | Ethereum, Base, Arbitrum, Optimism, Polygon, BNB Chain | ETH / BNB / etc. | ERC-20 (BEP-20 on BNB Chain) | One `Evm` account across all six; network selected per request, defaults to Ethereum mainnet. EIP-1559 / typed-tx signing. |
| Bitcoin | Mainnet | BTC | None | P2WPKH (native SegWit). **Rejects token transfers.** Esplora REST for balance/broadcast. |
| Solana | Mainnet / devnet (per RPC) | SOL | SPL | ed25519 signing; native + SPL token transfers. |
| Tron | Mainnet | TRX | TRC-20 | TronGrid REST for native + TRC-20 transfers. |

Built-in asset catalogs include the native asset plus common stablecoins per chain (e.g. USDC/USDT as ERC-20/BEP-20, USDC as SPL on Solana, USDT as TRC-20 on Tron).

***

## Onboarding and the recovery phrase

Setup is a single, all-or-nothing operation: it persists a consent flag, the mnemonic word count, the setup source, exactly one derived account per supported chain, and the encrypted recovery phrase. Valid BIP-39 mnemonic word counts are **12, 15, 18, 21, or 24**.

The recovery phrase is the only secret. The wallet stores the per-chain account **addresses** (safe to surface) separately from the secret material, and only the encrypted phrase can reconstruct private keys.

***

## Key custody and security

The wallet is **non-custodial and local**. There is no server-side key escrow.

- **The recovery phrase is always encrypted at rest.** It is encrypted via the core `encryption` domain before being persisted anywhere.
- **Preferred home: the OS keychain.** The encrypted phrase lives in the operating system keychain under the key `wallet.mnemonic`, scoped by a workspace-derived user id. Access is gated by the keyring consent policy.
- **Fallback: workspace JSON.** When the keychain is unavailable (e.g. headless), the encrypted phrase falls back to `{workspace_dir}/state/wallet-state.json`. On load, any JSON-resident secret is transparently **migrated into the keychain** when one becomes available, and stripped from the JSON.
- **Atomic, guarded writes.** `wallet-state.json` is written atomically (temp file + fsync + persist) under a process-wide lock. Corrupt or invalid state files are quarantined rather than trusted.
- **Decrypt only at signing time.** Chain signers decrypt the phrase in-core only when deriving a key to sign a confirmed transaction. Plaintext keys are never persisted and never serialized over the wire.

See [OS keyring & secret storage](os-keyring-and-secret-storage.md) for how secrets are stored across platforms, and [Privacy & security](privacy-and-security.md) for the broader model.

***

## Reading balances and chain info

Read-only surfaces require no confirmation:

- **Status**: onboarding state plus the safe per-chain account addresses.
- **Balances**: native-asset balances per account. Note: only **EVM balances read live** today (Ethereum mainnet); BTC, Solana, and Tron call their providers but fall back to a zero balance with a "provider missing" status on error.
- **Network defaults / supported assets**: per-chain RPC and explorer URLs, capability flags, and the built-in asset catalog.
- **Chain status**: per-chain readiness and the active RPC URL.

RPC endpoints are overridable per chain/network via `OPENHUMAN_WALLET_RPC_*` environment variables. URLs are redacted to scheme + host in logs.

***

## Sending transfers: prepare → confirm → execute

Every write is a two-step, intentional flow. The wallet never sends in one shot.

1. **Prepare** (`prepare_transfer`): validates the amount, destination address, and (for tokens) calldata, estimates fees, and returns a **prepared quote** with a `quoteId`. Quotes are held in an in-memory store with a **5-minute TTL**, capped at 64, and are **not** persisted across restarts.
2. **Confirm + execute** (`execute_prepared`): requires `confirmed: true` and a valid `quoteId`. The quote is consumed atomically before broadcast so concurrent confirmations can't double-submit; on failure it's restored with a refreshed TTL so it stays retryable.

**Quote-owner binding.** Each quote is bound to the chat thread that prepared it. A quote can only be executed by the same owner that prepared it; a `quoteId` leaked into a shared channel returns an indistinguishable "not found" error rather than letting another session hijack it.

Transfers are limited to **native sends and the token standards in the table above**. Bitcoin rejects token transfers. Swaps, bridges, and generic contract calls are not available here.

***

## Transaction status tracking

After broadcast, three read-only inspectors let the agent follow a transaction by hash:

- **`tx_status`**: lifecycle state (pending / confirmed / failed / not found).
- **`tx_receipt`**: receipt details (success, fee, block).
- **`lookup_tx`**: the raw transaction payload.

***

## Agent tools and approval safety

The agent reaches the wallet through six tools:

| Tool | Purpose |
| --- | --- |
| `wallet_status` | Onboarding status + account addresses. |
| `wallet_chain_status` | Per-chain readiness + active RPC. |
| `wallet_prepare_transfer` | Build a validated, fee-estimated quote. |
| `wallet_tx_status` | Transaction lifecycle state by hash. |
| `wallet_tx_receipt` | Transaction receipt by hash. |
| `wallet_lookup_tx` | Raw transaction lookup by hash. |

Notice there is **no agent tool that executes a transfer.** The agent can prepare a quote, but actually moving funds (`execute_prepared`) goes through the RPC surface, where it must be explicitly confirmed and pass the owner-binding check. Combined with the prepare-then-confirm flow and the per-thread quote binding, this keeps the agent from silently spending funds.

Because these are financial actions, they should be surfaced through the [approval gate](approval-gate.md) so a human confirms before money moves. Treat every transfer as a high-stakes action.

***

## See also

- [Approval gate](approval-gate.md)
- [Privacy & security](privacy-and-security.md)
- [OS keyring & secret storage](os-keyring-and-secret-storage.md)
