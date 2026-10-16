# Web3

This domain exposes wallet operations, token swaps, cross-chain bridges, EVM
dapp calls, and x402 payments. OpenHuman owns the product RPC and tool adapters,
configuration, credential custody, approvals, and generic HTTP execution. The
compiled TinyWallet module owns chain algorithms, provider-specific quote and
payment behavior, signing, payment accounting, and user-scoped service state.

Callers are the controller registry (`core/all.rs`), the tool registry
(`tools/ops.rs`), and the generic `http_request` tool, which invokes the x402
payment hook when a response contains an HTTP 402 challenge.

## Module composition

The host loads an attested TinyWallet artifact through `modules::wallet`, then
uses typed TinyBus operations for wallet status, transaction construction,
quote preparation and execution, x402 payment preparation and settlement, and
payment history. The host never runs wallet or payment algorithms as a local
fallback.

The host callbacks are narrow:

| Callback | Host responsibility |
| --- | --- |
| Wallet accounts | Return stored public account addresses. |
| Chain transport | Apply endpoint configuration and generic HTTP/RPC safety, then return the response. |
| Wallet backend | Answer authenticated wallet-backend requests through the registered handler (`modules::wallet_backend`). |
| Web3/native/x402 authorization | Apply OpenHuman's explicit user approval policy to the exact module-provided intent. |

Secrets are decrypted from host custody only for a confidential call to the
attested module. The host retains encrypted wallet persistence and user consent;
the module derives keys and performs signing. User sign-out and shutdown close
the module service handles, clearing quote and payment state before the host
discards endpoint references.

## Swap, bridge, and dapp operations

The compiled module creates quotes and binds them to the active chat thread and
client. The host exposes the existing RPC namespaces and forwards request and
response values, without deriving chains, rebuilding transactions, or signing
on the host. Execution requires `confirmed: true`; the module validates quote
ownership and gives the host the exact intent for approval at the signer seam.

The backend deBridge proxy is reached through the wallet-backend hook
(`modules::wallet_backend`); `openhuman-tinyhumans` answers it over
`/agent-integrations/crypto/*`, and a core with none registered answers
`BACKEND_UNAVAILABLE:`. Provider response parsing, chain-family
selection, quote construction, transaction encoding, and quote storage are
module-owned. Provider-specific URLs and defaults stay in the module; the host
passes only approved endpoint overrides through opaque endpoint references.

## Wallet and x402

The wallet module owns account derivation, supported assets, network defaults,
balance and transaction queries, transfer preparation and execution, address
validation, ERC-20 calldata encoding, and wallet agent operations. OpenHuman
retains onboarding, encrypted recovery-phrase storage, user consent, and host
approval.

For x402, OpenHuman's generic HTTP tool retains domain allowlisting, request
policy, user approval, and HTTP execution. On an HTTP 402, the host sends the
challenge and host-custodied encrypted wallet secrets over the confidential
module call. TinyWallet parses the challenge, chooses the payment chain,
reserves its own budget, signs, and returns the payment header. The host retries
the HTTP request, then reports the raw response status and settlement header
back to the module. The module owns settlement parsing, reservation commit or
release, the ledger, budgets, and payment history. The host does not keep a
second x402 ledger or parse payment protocol payloads.

## Layout

| Path | Responsibility |
| --- | --- |
| [`mod.rs`](./mod.rs) | Web3 facade and controller/tool aggregation. |
| [`client.rs`](./client.rs) | Generic authenticated client for the deBridge integration proxy. |
| [`module_tools.rs`](./module_tools.rs) | Agent-tool adapters over TinyWallet bus operations. |
| [`swap/`](swap/README.md), [`bridge/`](bridge/), [`dapp/`](dapp/README.md) | Product RPC schemas and controller handlers. |
| [`wallet/`](wallet/README.md) | Wallet onboarding, host config/custody, controller and tool adapters. |
| [`x402/`](x402/README.md) | x402 controller projection over the module-owned ledger. |
| [`../modules/wallet.rs`](../modules/wallet.rs) | TinyBus identity, attestation, and typed calls. |
| [`../modules/wallet_host.rs`](../modules/wallet_host.rs) | Host callbacks for custody, transport, and exact-intent approval. |
| [`../modules/web3_service.rs`](../modules/web3_service.rs) and [`../modules/x402_service.rs`](../modules/x402_service.rs) | User-scoped module service setup and cleanup. |

## RPC and agent tools

All controllers are registered under `DomainGroup::Web3` in `core/all.rs`.
Namespace names are wire contracts.

| Namespace | Operations |
| --- | --- |
| `web3_swap` | `quote`, `execute`, `routes` |
| `web3_bridge` | `quote`, `execute` |
| `web3_dapp` | `call`, `execute` |
| [`wallet`](./wallet/README.md) | `setup`, `status`, `reveal_recovery_phrase`, `balances`, `chain_status`, `network_defaults`, `supported_assets`, `encode_erc20_transfer`, `prepare_transfer`, `execute_prepared`, `tx_status`, `tx_receipt`, `lookup_tx` |
| [`x402`](./x402/README.md) | `get_summary`, `list_payments`, `update_budget` |

The existing Wallet/Web3/x402 agent declarations come from `tinywallet-bus`;
OpenHuman supplies adapters that call the module. The `x402_request` tool uses
the generic host HTTP executor and its payment hook, retaining its existing
name and parameter schema.

## Boundaries and validation

- TinyWallet owns wallet, Web3, and x402 algorithms. OpenHuman owns product
  configuration, encrypted secret custody, approval policy, generic HTTP/RPC
  transport, and controller compatibility.
- Module calls fail closed when the artifact or required operation is
  unavailable or incompatible. There is no local implementation fallback.
- `web3` is included in `scripts/ci/product-features.txt`; `DomainSet::Web3`
  gates the same surface at runtime. The `web3` Cargo feature adds the bus
  contract and module support, not TinyWallet implementation crates.
- The `client` logs endpoint and operation metadata only; request bodies and
  wallet addresses must not be logged.

```bash
cargo test -p openhuman --features web3 web3::
cargo check -p openhuman --no-default-features
node scripts/ci/check-module-boundaries.mjs
```

## Further reading

- [Wallet](../../../../docs/gitbooks/en/features/wallet.md)
- [TinyWallet](../../../../vendor/tinywallet/README.md)
- [Loadable modules](../../../../docs/gitbooks/en/developing/loadable-modules.md)
