# x402

OpenHuman exposes x402 payment history and budget RPCs and connects payment
handling to its generic HTTP tool. TinyWallet owns challenge parsing, chain and
asset selection, payment encoding and signing, budget reservation, settlement
parsing, and ledger persistence.

The host keeps generic HTTP policy: allowed destinations, request authorization,
network execution, and explicit approval. It retrieves encrypted wallet secrets
from host custody and sends them only through the confidential TinyWallet
module call. The module returns the payment header; after the retry, the host
passes the raw response status and settlement header back to the module. The
host does not parse x402 payloads or keep a parallel ledger.

Payments are available through both the `x402_request` tool and the generic
`http_request` tool's HTTP 402 payment hook. Both use the same module-owned
budget and ledger.

## Compile-time gate

`x402` is an always-available facade. Its real controllers require the `web3`
feature, while `stub.rs` provides empty controller lists when disabled. The
product enables `web3` through `scripts/ci/product-features.txt`.

## Host files

| File | Responsibility |
| --- | --- |
| [`mod.rs`](./mod.rs) | Facade, payment record vocabulary re-exports, and disabled stub. |
| [`schemas.rs`](./schemas.rs) | Stable `openhuman.x402_*` controller names and RPC projections. |
| [`schemas_tests.rs`](./schemas_tests.rs) | Pins payment history output, amount display, and UTC timestamp representation. |
| [`../../modules/x402_service.rs`](../../modules/x402_service.rs) | User-scoped module configuration, confidential payment calls, history and budget calls, and close/reconfigure cleanup. |
| [`../../tools/impl/network/host.rs`](../../tools/impl/network/host.rs) | Generic HTTP payment hook; sends the challenge and returns the module-produced payment header. |

## RPC surface

| Function | Purpose |
| --- | --- |
| `get_summary` | Project module-owned spending totals and configured limits. |
| `list_payments` | Project recent module-owned payment records using the existing RPC shape. |
| `update_budget` | Ask the module to update per-request, daily, and monthly limits. |

Payment history keeps the established fields and formatting, including
`amount_display` and UTC timestamps. These are host projections of module DTOs,
not host-side accounting or protocol interpretation.

## Safety and lifecycle

- The host rejects a request before secret access if user approval is absent.
- The module parses the challenge, selects the payment rail, reserves budget,
  and verifies the exact owner and payment facts before signing.
- The module service is closed on sign-out, configuration replacement, and
  shutdown. Pending reservations are released by the module before its ledger
  is replaced or discarded.
- If the module or required capability is unavailable, payment fails closed;
  the host does not fall back to a local payment implementation.

## Further reading

- [Parent Web3 module](../README.md)
- [Wallet host adapter](../wallet/README.md)
- [TinyWallet source](../../../../../vendor/tinywallet/README.md)
- [Loadable modules](../../../../../docs/gitbooks/en/developing/loadable-modules.md)
