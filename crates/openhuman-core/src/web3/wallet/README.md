# Wallet

OpenHuman owns wallet onboarding, consent, encrypted mnemonic custody, and the
product RPC surface. The compiled TinyWallet module owns account derivation,
address validation, chain defaults and reference data, transaction building,
signing, ERC-20 encoding, provider access, and user-scoped quote state.

The host decrypts a mnemonic only for confidential calls to the attested module
and stores only the encrypted phrase. It applies explicit user approval to the
exact prepared transaction before the module signer runs. Missing or
incompatible module operations fail closed; there is no host implementation
fallback.

## Compile-time gate

`wallet` remains an always-available facade. The real implementation is gated
by the `web3` Cargo feature; `stub.rs` provides the disabled surface for
always-on callers. The product enables `web3` through
`scripts/ci/product-features.txt`. Run both enabled and disabled builds after
changing the facade.

## Host responsibilities

- `ops/` persists consent, setup source, word count, derived public accounts,
  and encrypted mnemonic state. It prefers the OS keychain and retains the
  workspace JSON fallback for headless operation.
- `execution.rs` maps typed TinyWallet calls into controller results.
- `endpoints.rs` reads host endpoint overrides and the Solana cluster. The
  module chooses defaults and returns only opaque references for host-provided
  endpoints.
- `rpc.rs` is the generic host HTTP/RPC transport used by guarded module
  callbacks; it redacts endpoint details in logs.
- `schemas.rs` preserves the `openhuman.wallet_*` RPC names and response
  shapes.
- `tools.rs` adapts the static tool declarations from `tinywallet-bus` to
  OpenHuman's `tinytools::Tool` interface.
- `modules::wallet_service` manages the native wallet service handle and
  closes it on replacement, sign-out, and shutdown.

## RPC surface

| Function | Purpose |
| --- | --- |
| `status`, `setup`, `reveal_recovery_phrase` | Onboarding, public account metadata, encrypted phrase custody, and explicit reveal. |
| `balances`, `chain_status`, `network_defaults`, `supported_assets` | Read module-owned provider and network data. |
| `encode_erc20_transfer` | Request standard EVM transfer calldata from the module. |
| `prepare_transfer`, `execute_prepared` | Prepare a module-owned quote, then explicitly confirm and execute it. |
| `tx_status`, `tx_receipt`, `lookup_tx` | Read transaction lifecycle and raw transaction data through the module. |

The four chain families are EVM, Bitcoin, Solana, and Tron. EVM network
selection, asset catalogs, explorers, mint metadata, transaction encoding, and
provider-specific behavior remain module-owned. OpenHuman supplies only
explicit endpoint overrides, authenticated host transport, custody, and
approval policy.

## Agent tools

The six wallet tools are declared in `tinywallet-bus` and adapted locally:
`wallet_status`, `wallet_chain_status`, `wallet_prepare_transfer`,
`wallet_tx_status`, `wallet_tx_receipt`, and `wallet_lookup_tx`. Transfer
execution remains behind the controller confirmation flow.

## Persistence and lifecycle

- `{workspace_dir}/state/wallet-state.json` stores consent and public setup
  metadata, with the encrypted mnemonic only when the OS keychain is
  unavailable. Writes remain atomic and corrupt state is quarantined.
- The OS keychain is preferred for the encrypted mnemonic when policy and
  platform support permit it.
- Prepared transactions and one-shot approval state live in the module service
  and are not persisted across sign-out or process restart. Closing the service
  invalidates quotes and endpoint aliases before the host discards its config.

## Tests and further reading

Wallet behavior and cryptographic execution are tested in the owning TinyWallet
repository and against its compiled module artifact. OpenHuman tests cover
onboarding persistence, host approval, adapter behavior, and controller/tool
compatibility.

- [Parent Web3 module](../README.md)
- [Wallet product guide](../../../../../docs/gitbooks/en/features/wallet.md)
- [TinyWallet source](../../../../../vendor/tinywallet/README.md)
- [Loadable modules](../../../../../docs/gitbooks/en/developing/loadable-modules.md)
