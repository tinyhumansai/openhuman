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

Payments are available through the `x402_request` tool, the generic
`http_request` tool's HTTP 402 payment hook, and the `openhuman.x402_fetch` RPC
for a local caller with no agent turn. All use the same module-owned budget and
ledger.

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
| [`fetch.rs`](./fetch.rs) | `x402_fetch`: parameter validation, guarded HTTPS request, capped payment, retry, settlement report, response projection. |
| [`fetch_tests.rs`](./fetch_tests.rs) | Pins `x402_fetch` parameter refusals, the destination guard, body truncation, the refusal prefix, and the response shape. |
| [`../../modules/x402_service.rs`](../../modules/x402_service.rs) | User-scoped module configuration, confidential payment calls, history and budget calls, and close/reconfigure cleanup. |
| [`../../tools/impl/network/host.rs`](../../tools/impl/network/host.rs) | Generic HTTP payment hook; sends the challenge and returns the module-produced payment header. |

## RPC surface

| Function | Purpose |
| --- | --- |
| `get_summary` | Project module-owned spending totals and configured limits. |
| `list_payments` | Project recent module-owned payment records using the existing RPC shape. |
| `update_budget` | Ask the module to update per-request, daily, and monthly limits. |
| `fetch` | Make one HTTPS request and pay an HTTP 402 challenge within the caller's `maxAmountAtomic`. |

### `fetch`

For a local, non-model caller (for example a script over loopback JSON-RPC)
that has no agent turn to approve a payment through.

Params: `url` (required, `https://` only; local and private hosts are refused
by the `http_request` URL guard, including names that resolve to them),
`method` (`GET` default, or `POST`), `headers` (string map; `PAYMENT-SIGNATURE`
and `X-PAYMENT` are refused), `body` (string, `POST` only), and
`maxAmountAtomic` (required, greater than zero: the most this request may pay).

The request runs with a 30 s timeout, no redirects, and connections pinned to
the vetted addresses; the privacy mode's egress check applies. A response other
than a 402 with a `PAYMENT-REQUIRED` challenge is returned unpaid. On a
challenge the host calls the module's prepare with a one-shot permit installed
(`modules::wallet_host::with_x402_rpc_permit`) while it holds the x402 service
lock. The authorization callback approves only when that permit is active for
the exact URL the module reports, the amount is within `maxAmountAtomic`, and
it is within the per-request budget; the permit is consumed by the first
callback. Without a permit, routing is unchanged. The module still reserves its
own per-request, daily and monthly budget first. The host retries once with the
payment header and always reports the retry status and `PAYMENT-RESPONSE`
header back to the module, also when the retry fails.

Response: `{status, contentType?, body, truncated, payment}`. `body` is UTF-8
text cut at 256 KiB (`truncated: true`). `payment` is `null` when nothing was
paid, else `{paymentId, amountAtomic, asset, recipient, network, settled,
settlementResponse?}`, where `settled` means the retry was 2xx, carried a
settlement header, and the module ledger recorded the payment as settled.

A payment over `maxAmountAtomic` or the per-request budget, or otherwise
refused by the host's authorization, is an error starting with
`X402_REFUSED:`. A module-side budget refusal (daily or monthly) and other
payment failures start with `x402_fetch: payment failed:`.

Payment history keeps the established fields and formatting, including
`amount_display` and UTC timestamps. These are host projections of module DTOs,
not host-side accounting or protocol interpretation.

## Safety and lifecycle

- The module reserves the amount against its budget, then asks the host to
  authorize (`modules::wallet_host::x402_rpc_permit_route`, then
  `x402_route`). A direct `x402_fetch` RPC holding a permit for the URL is
  approved within its cap and the per-request budget. A WebChat turn gets a forced
  approval prompt in its own chat; trusted automation (a background job or an
  approval-free workflow) is approved without a person when the amount is
  within the per-request budget; an external channel, an unlabelled turn and
  anything else are refused.
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
