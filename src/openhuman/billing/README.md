# billing

Thin RPC adapter domain over the hosted backend's payment API. It exposes plan lookup, Stripe/Coinbase purchase and top-up flows, credit balance/transactions, auto-recharge + saved-card management, and coupon redemption through the standard controller registry (`openhuman.billing_*`). It holds **no payment logic or state of its own** — every operation forwards an authenticated HTTPS request to the backend (`/payments/*`, `/coupons/*`) and surfaces the JSON response verbatim. Authorization, plan ownership, tenant isolation, and payment policy are enforced backend-side.

## Responsibilities

- Authenticate each call with the stored app-session JWT (`Authorization: Bearer …`) and reject calls with no session.
- Fetch current plan/entitlements (`/payments/stripe/currentPlan`) and credit balance (`/payments/credits/balance`).
- Create Stripe Checkout sessions (`purchase_plan`), the Stripe customer portal session, and Stripe SetupIntents for adding cards.
- Initiate credit top-ups via Stripe or Coinbase, and create Coinbase Commerce charges (crypto / annual billing).
- Page credit transaction history; read/update Stripe auto-recharge settings; list/update/delete saved cards.
- Redeem coupon codes and list the current user's redeemed coupons.
- Do **pre-HTTP input validation** (non-empty plan/code/paymentMethodId, finite positive `amountUsd`, gateway whitelist `stripe`/`coinbase`) before any network call.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/billing/mod.rs` | Export-focused module root; re-exports `ops::*` and the schemas/controllers pair. |
| `src/openhuman/billing/ops.rs` | Business logic: one async fn per backend endpoint; auth helper (`require_token`, `get_authed_value`); input validation + gateway normalization. Returns `RpcOutcome<Value>`. |
| `src/openhuman/billing/schemas.rs` | Controller schemas, `all_billing_controller_schemas` / `all_billing_registered_controllers`, param structs, and `handle_billing_*` handlers delegating to `ops`. |
| `src/openhuman/billing/schemas_tests.rs` | Sibling test suite for `schemas.rs` (wired via `#[path]` mod). |

## Public surface

From `mod.rs`:

- `ops::*` — async handlers: `get_current_plan`, `get_balance`, `get_transactions`, `get_auto_recharge`, `update_auto_recharge`, `get_cards`, `create_setup_intent`, `update_card`, `delete_card`, `purchase_plan`, `create_portal_session`, `top_up_credits`, `create_coinbase_charge`, `redeem_coupon`, `get_user_coupons`. Each takes `&Config` (plus typed params) and returns `Result<RpcOutcome<Value>, String>`.
- `all_billing_controller_schemas()`, `all_billing_registered_controllers()`, `billing_schemas(function: &str)` — registry wiring.

## RPC / controllers

Namespace `billing` (15 methods, exposed as `openhuman.billing_*`):

| Method | Backend endpoint |
| --- | --- |
| `billing_get_current_plan` | `GET /payments/stripe/currentPlan` |
| `billing_get_balance` | `GET /payments/credits/balance` |
| `billing_get_transactions` | `GET /payments/credits/transactions?limit&offset` |
| `billing_get_auto_recharge` | `GET /payments/credits/auto-recharge` |
| `billing_update_auto_recharge` | `PATCH /payments/credits/auto-recharge` |
| `billing_get_cards` | `GET /payments/credits/auto-recharge/cards` |
| `billing_create_setup_intent` | `POST /payments/credits/auto-recharge/cards/setup-intent` |
| `billing_update_card` | `PATCH /payments/credits/auto-recharge/cards/{paymentMethodId}` |
| `billing_delete_card` | `DELETE /payments/credits/auto-recharge/cards/{paymentMethodId}` |
| `billing_purchase_plan` | `POST /payments/stripe/purchasePlan` |
| `billing_create_portal_session` | `POST /payments/stripe/portal` |
| `billing_top_up` | `POST /payments/credits/top-up` |
| `billing_create_coinbase_charge` | `POST /payments/coinbase/charge` |
| `billing_redeem_coupon` | `POST /coupons/redeem` |
| `billing_get_coupons` | `GET /coupons/me` |

Handlers load `Config` via `config::rpc::load_config_with_timeout()`, deserialize camelCase params, call the matching `ops` fn, and emit CLI-compatible JSON via `RpcOutcome::into_cli_compatible_json()`.

## Agent tools

None. This domain has no `tools.rs`; it is RPC/CLI-only.

## Events

None. No `bus.rs`; the domain neither publishes nor subscribes to `DomainEvent`s.

## Persistence

None of its own; stateless adapter. The only state it reads is the backend session JWT, which lives in the `api` layer (see `auth_store_session` / `get_session_token`), not in this module.

## Dependencies

- `crate::api::config::effective_backend_api_url` — resolves the backend base URL from `config.api_url`.
- `crate::api::jwt::get_session_token` — reads the stored app-session JWT.
- `crate::api::BackendOAuthClient` — performs the authenticated JSON HTTP request (`authed_json`).
- `crate::openhuman::config::Config` — config struct (`api_url`); `config::rpc::load_config_with_timeout` in handlers.
- `crate::rpc::RpcOutcome` — standard RPC return/logging wrapper.
- `crate::core::all::{ControllerFuture, RegisteredController}` and `crate::core::{ControllerSchema, FieldSchema, TypeSchema}` — controller registry types.
- External crates: `reqwest` (`Method`), `serde`/`serde_json`, `urlencoding` (path-segment encoding for `paymentMethodId`).

## Used by

- `src/core/all.rs` — registers `all_billing_registered_controllers()` (controllers, ~L215) and `all_billing_controller_schemas()` (schemas, ~L346) into the global registry. This is the sole in-tree consumer.

## Notes / gotchas

- **No local authorization** — security is delegated entirely to the backend; a missing/invalid session yields the backend 401/403 surfaced verbatim as an RPC error string. JWTs/API keys are never logged.
- `top_up_credits` and `create_coinbase_charge` default gateway/interval (`stripe`, `annual`); `normalize_gateway` lowercases and whitelists `stripe`/`coinbase`, treating empty/whitespace as default-to-stripe.
- `amountUsd` must be finite and `> 0` (NaN/±Inf/≤0 rejected pre-HTTP).
- `paymentMethodId` is `urlencoding::encode`'d into the path; empty/whitespace ids are rejected.
- `get_transactions` defaults to `limit=20`, `offset=0`; the handler tolerates an empty params map.
- Input-validation unit tests live inline in `ops.rs` and run without network/session/filesystem state.
