# hosted

RPC proxy domains for the hosted TinyHumans backend (`tinyhumansai/backend`).
Each one is a thin client: the truth lives server-side, and this side only
authenticates a request, forwards it and shapes the result. They live in this
crate rather than the core because a core with no TinyHumans connection has
no use for them. `crate::install()` registers them into whichever core
registry the host boots, so the RPC methods appear without the core knowing
these domains exist.

## How it works

### Registration

`extension()` in [`mod.rs`](mod.rs) collects every domain's
`all_<domain>_registered_controllers()` into one `ControllerExtension` tagged
`DomainGroup::Hosted`, together with the `NAMESPACES` table that
`namespace_description` serves for the namespaces only this surface uses
(`billing`, `team`, `referral`, `announcements`). `crate::install()` passes
the extension to `core::all::register_controller_extension`; the crate root
also re-exports it as `hosted_controllers` for hosts that register it
themselves. [`crates/openhuman-core/src/core/all.rs`](../../../openhuman-core/src/core/all.rs) is the extension point and
never grows a `billing` or `team` branch of its own.

Three domains (`webhooks`, `channel_link`, `oauth`) put methods into
namespaces the core also uses (`webhooks`, `channels`, `auth`). The registry
keys controllers by `namespace.function`, so a namespace can be split between
the core and this extension. The core keeps describing those shared
namespaces, which is why they are absent from `NAMESPACES`.

Because the controllers sit in `DomainGroup::Hosted`, a runtime whose
`DomainSet` excludes the hosted group hides them even when they are
registered.

### One call, end to end

```text
 JSON-RPC  openhuman.billing_get_balance
   |
   v
 schemas.rs handler         load config (DispatchCtx::load_config)
   |                        read and validate params
   v
 ops.rs operation           validate inputs (trimmed, non-empty) first
   |
   v
 HostedClient::from_config  resolve_backend_credential(config)
   |                        require an installed backend transport
   |                        base = origin of effective_backend_api_url
   |                        TinyHumansClient + Bearer or x-api-key
   |
   +-- no credential or no transport: return the core's sentinel,
   |   with no request
   v
 client.sdk().billing().<typed method>().await
   |
   v
 client.finish(op, result)  SDK error -> RPC error string (see below)
   |
   v
 Outcome<Value> -> into_cli_compatible_json -> JSON-RPC result
```

Every domain follows that shape: an `ops.rs` of async operations returning
`Result<Outcome<Value>, String>`, and a `schemas.rs` that declares the
`ControllerSchema`s and registers thin handlers. None of them own local
state, domain types or authorization; the backend enforces ownership, roles
and tenant isolation.

### HostedClient (`client.rs`)

`HostedClient` is the one way a hosted domain reaches the backend. It checks
for a credential before anything touches the network: the offline local
session, a missing token and a locally expired token all return the core's
own error string, which already carries the right sentinel
(`BACKEND_UNAVAILABLE:` or `SESSION_EXPIRED:`). A user without a TinyHumans
account therefore never produces a doomed request or a Sentry event. With no
transport installed it returns `BACKEND_UNAVAILABLE: no backend transport
installed`.

The client strips the configured API URL to its origin (so a
completions-style `api_url` still reaches `/teams/...`), reuses the installed
transport's `reqwest::Client` (or builds one with the same settings) and
stamps the product identity as SDK default headers. A session JWT goes out as
a bearer token and an API key as `x-api-key`; `HostedClient::kind()` records
which.

`finish` and `finish_value` map an SDK error onto the RPC string channel in
one place:

| SDK error | RPC error string |
| --- | --- |
| `Status { 401 }` with a session | `SESSION_EXPIRED: backend rejected session token on {op}` |
| `Status { 401 }` with an API key | `API_KEY_REJECTED: backend rejected api key on {op}` |
| other `Status` | `{op} failed ({status}): {body}` |
| `Http` | `backend request {op}: {error and its source chain}` |
| `Envelope`, `Decode`, others | `backend request {op}: {error}` |
| `RouteNotExposed` | same shape, logged at `error` because it is a bug here |

`SESSION_EXPIRED:` makes the JSON-RPC layer skip Sentry and publish a
session-expired event so the host re-signs in. An API key has no session to
expire, so it maps to `API_KEY_REJECTED:` instead. The other shapes match what
`BackendClient::authed_json` produces, so the JSON-RPC classifiers (transient
status, transport phrases, budget exhaustion) keep matching them.

## Layout

| Path | What it does |
| --- | --- |
| [`mod.rs`](mod.rs) | `extension()`, `NAMESPACES`, module declarations. |
| [`client.rs`](client.rs) | `HostedClient`, `CredentialKind`, `map_error`. |
| [`billing/`](billing/README.md) | Plans, Stripe and Coinbase purchase and top-up flows, credit balance and transactions, auto-recharge, saved cards, coupons (`/payments/*`, `/coupons/*`). |
| [`team/`](team/README.md) | Team CRUD, membership, role changes, invites, usage (`/teams/me/usage`), active-team switching. |
| [`referral/`](referral/README.md) | Referral stats and claiming a code (`/referral/*`). |
| [`announcements/`](announcements/README.md) | The latest active product announcement (`GET /announcements/latest`); a 404 folds into "no announcement". |
| [`webhooks/`](webhooks/README.md) | Backend-managed webhook tunnel CRUD and the bandwidth budget (`/webhooks/core*`). |
| [`channel_link/`](channel_link/README.md) | Linking the managed Telegram and Discord bots to the account: link-token issuance and the start/check flows. `managed.rs` polls `GET /auth/me` and stores a `channel:<id>:managed_dm` credential marker on success. |
| [`oauth/`](oauth/README.md) | Backend-brokered OAuth integrations (`/auth/{provider}/connect`, `/auth/integrations*`). `handoff.rs` decrypts the AES-256-GCM token handoff. |
| [`test_support.rs`](test_support.rs) | Wiremock fixtures shared by this tree's tests (test builds only). |

## RPC surface

Wire names are `openhuman.<namespace>_<function>` and are unchanged wire
contracts.

| Domain | Methods |
| --- | --- |
| `billing` | `billing_get_summary`, `billing_get_current_plan`, `billing_get_balance`, `billing_purchase_plan`, `billing_create_portal_session`, `billing_top_up`, `billing_create_coinbase_charge`, `billing_get_transactions`, `billing_get_auto_recharge`, `billing_update_auto_recharge`, `billing_get_cards`, `billing_create_setup_intent`, `billing_update_card`, `billing_delete_card`, `billing_redeem_coupon`, `billing_get_coupons` |
| `team` | `team_get_usage`, `team_list_members`, `team_list_teams`, `team_get_team`, `team_create_team`, `team_update_team`, `team_delete_team`, `team_switch_team`, `team_leave_team`, `team_join_team`, `team_create_invite`, `team_remove_member`, `team_change_member_role`, `team_list_invites`, `team_revoke_invite` |
| `referral` | `referral_get_stats`, `referral_claim` |
| `announcements` | `announcements_get_latest` |
| `webhooks` | `webhooks_list_tunnels`, `webhooks_create_tunnel`, `webhooks_get_tunnel`, `webhooks_update_tunnel`, `webhooks_delete_tunnel`, `webhooks_get_bandwidth` |
| `channel_link` | `auth_create_channel_link_token`, `channels_telegram_login_start`, `channels_telegram_login_check`, `channels_discord_link_start`, `channels_discord_link_check` |
| `oauth` | `auth_oauth_connect`, `auth_oauth_list_integrations`, `auth_oauth_fetch_integration_tokens`, `auth_oauth_revoke_integration`, `auth_oauth_fetch_client_key` |

## Boundaries

- The core keeps the controllers that share these namespaces: the local
  webhook router (`webhooks_list_registrations`, `register_echo`,
  `trigger_agent`, and so on), the credential controllers in `auth`, and the
  rest of `channels`. The `channels.*` schemas used by `channel_link` come
  from the `tinychannels-bus` contract through the core's
  `channels::contract_schema`.
- Routes come from the vendored SDK ([`vendor/tinyhumans-sdk`](../../../../vendor/tinyhumans-sdk/)). Where the SDK
  has no typed method, a domain uses the SDK's raw request primitive
  (`billing`, and `oauth`'s `POST /auth/integrations/{id}/client-key`).
  `team`'s `POST /teams` and `DELETE /teams/{id}` are not in the SDK's route
  registry at all, so they stay on this crate's `BackendClient::authed_json`
  until the SDK carries them.
- These domains do not go through `BackendTransport::send_json`; they build a
  typed `TinyHumansClient` directly. They still depend on this crate's shared transport, for the gate and for its
  HTTP client.

## Gotchas

- Validate inputs before building the `HostedClient`, and build the client
  before any request. The test suites pin that order: a user without an
  account must get the core's sentinel with no network traffic.
- Never log tokens or link tokens. `channel_link` logs link tokens by length
  only.
- New hosted-only proxy domains belong here, wired into `extension()`, not in
  the core.

## Tests

Each domain has `ops_tests.rs` and `schemas_tests.rs` beside its modules
(plus `managed_tests.rs` and `handoff_tests.rs` where those files exist), and
[`client_tests.rs`](client_tests.rs) covers `HostedClient` and the error mapping. They run
against wiremock through [`test_support.rs`](test_support.rs).

```bash
cargo test -p openhuman-tinyhumans hosted::
```

## Further reading

- [`crates/openhuman-tinyhumans/src/hosted/announcements/README.md`](announcements/README.md): the announcements module README.
- [`crates/openhuman-tinyhumans/src/hosted/billing/README.md`](billing/README.md): the billing module README.
- [`crates/openhuman-tinyhumans/src/hosted/channel_link/README.md`](channel_link/README.md): the channel_link module README.
- [`crates/openhuman-tinyhumans/src/hosted/oauth/README.md`](oauth/README.md): the oauth module README.
- [`crates/openhuman-tinyhumans/src/hosted/referral/README.md`](referral/README.md): the referral module README.
- [`crates/openhuman-tinyhumans/src/hosted/team/README.md`](team/README.md): the team module README.
- [`crates/openhuman-tinyhumans/src/hosted/webhooks/README.md`](webhooks/README.md): the webhooks module README.
- [`docs/gitbooks/en/developing/tinyhumans-api-key.md`](../../../../docs/gitbooks/en/developing/tinyhumans-api-key.md): running on a TinyHumans API key.
