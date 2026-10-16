# openhuman-tinyhumans

This crate connects an OpenHuman core to the hosted TinyHumans backend and owns
everything hosted. The core runs agents, memory, tools and RPC without any
backend connection and has no backend module, hosted port, hosted route or
hosted hostname. This crate holds the transport port and its implementation on
the vendored `tinyhumans-sdk`, the authenticated client, the routes, and the
hosted providers (integrations, voice, inference, channels, memory), and plugs
them into the core's generic registries. It also adds the pieces that only make
sense for a TinyHumans-connected product: the hosted RPC proxies (billing,
team, referral and so on), the host-side login and session owner, and the
Jev-backed `tool_search` ranker.

Every host that boots a core and wants the backend calls into this crate first:
the desktop shell ([`crates/openhuman-app`](../openhuman-app/)), the TUI ([`crates/openhuman-tui`](../openhuman-tui/)),
the CLI binary ([`crates/openhuman-cli`](../openhuman-cli/)) and the integration-test fixtures.
Library products build through its `RuntimeBuilder`.

## How it works

### Where it sits

```text
  openhuman-app    openhuman-tui    openhuman-cli    library products
        |                |                |                 |
        +----------------+-------+--------+-----------------+
                                 |  install() or RuntimeBuilder
                                 v
  +------------------------------------------------+
  |              openhuman-tinyhumans              |
  | transport/ client/ backend/ hosted/ session/ jev/|
  | integrations/ voice/ inference/ channels/ memory/|
  +------------------------------------------------+
           |                               |
           v                               v
    openhuman-embed               vendor/tinyhumans-sdk
  (Runtime -> Agent facade)       (HTTP, route registry,
           |                       auth header shapes)
           v
    openhuman-core
  (generic registries, controller registry)
```

The core has no `tinyhumans-sdk` dependency, and `cargo tree -p openhuman -i
tinyhumans-sdk` must stay empty. This is the only crate in the tree allowed to
depend on the SDK. A core with this layer not connected still works; every
backend-touching call (billing, integrations tools, channel relay, cloud
voice) answers with `HostedApiError::BackendUnavailable`, which the RPC layer
renders as a `BACKEND_UNAVAILABLE:` error that observability demotes.

The crate's only openhuman dependency is `openhuman-embed`, per the chain
core -> embed -> tinyhumans -> rpc -> hosts. The public embed API covers the seam types
(`embed::seams::{ControllerExtension, ...}`; the ranker trait is `tinytools::ToolRanker`, taken from the same vendored path); the deeper core
internals the transport and hosted controllers need (the registration
functions of the core's generic registries, `core::all`'s registry types, `security::credentials`, `config`) come through
embed's `#[doc(hidden)] __host` module, an explicit list kept in embed.

### Installing into a process

`install(InstallOptions)` in [`src/install.rs`](src/install.rs) is the compatibility entry point
for hosts that still boot the core themselves. It resolves the same `Wiring`
(`install::wiring`) the builder uses and applies it to the process globals.
It does the following, in this order:

1. If `InstallOptions::product_identity` is set, it stores the new
   `ProductIdentity` (the `x-sdk-name` header value) and drops the shared
   transport it built earlier, because the transport captures attribution
   headers when it is constructed.
2. It plugs every area (`client`, `integrations`, `voice`, `inference`,
   `channels`, `memory`) into the core's generic registries through
   `register::register_all`.
3. If `hosted_controllers` is true (the default), it registers
   `hosted::extension()` with the core through
   `core::all::register_controller_extension`. The core treats an identical
   re-registration as a no-op.
4. With the `jev` feature and `tool_ranker` true (the default), it installs a
   `jev::TinyHumansJevRanker` as the process-wide `tool_search` ranker,
   replacing the previous one in place.
5. It builds the one process-wide `SdkBackendTransport`
   (`client::shared_transport`). The core has no transport slot; every hosted
   caller in this crate sends through this instance. A later call reuses it.

It does five things, in this order. `install` returns the
`Arc<SdkBackendTransport>`. `is_installed()` reports whether `install` (or the
builder's connect) has run in this process.

```rust
openhuman_tinyhumans::install(openhuman_tinyhumans::InstallOptions::default())?;
```

The CLI does this in [`crates/openhuman-cli/src/main.rs`](../openhuman-cli/src/main.rs) before
`run_core_from_args`; in-process test suites do it through
[`tests/support/tinyhumans_boot.rs`](../../tests/support/tinyhumans_boot.rs).

### The configuration path: RuntimeBuilder

`RuntimeBuilder` in [`src/runtime.rs`](src/runtime.rs) wraps `openhuman_embed::RuntimeBuilder`.
It starts from `new()`, a host preset (`library()`, `desktop()`, `cli()`,
`tui()`) or `from_embed()`, and forwards every embed knob unchanged
(`workspace`, `workspace_dir`, `action_dir`, `config_source`, `token`,
`listen`, `api_key`, `backend_url`, `provider`, `access`, `services`,
`domains`, `tool_groups`, `host_kind`, `session`, `config`, `session_store`,
`memory_engine`) and every seam option (`controller_extension`, `tool_ranker`,
`post_turn_hook`, `tool_hook`, `server_launcher`, `live_policy`). It adds
`product_identity`, `hosted_controllers` and `jev_ranker`, which feed
`InstallOptions`.

`connect()` returns the embed builder with the TinyHumans connection applied
through embed's own options: the product identity is set, the registries are filled, the shared SDK
transport is built, the hosted proxies go in with `controller_extension`, and the Jev ranker with
`tool_ranker` (restored when the runtime drops). A host `tool_ranker` replaces
the Jev one, however it was set. `build()` is `connect()` then boot; `run_from_args()` is
`connect()` then the embed CLI dispatcher. `into_embed()` drops back to the
plain embed builder with no backend connection.

```rust
use openhuman_tinyhumans::{embed::Workspace, RuntimeBuilder};

let runtime = RuntimeBuilder::new()
    .workspace(Workspace::Ephemeral)
    .api_key("th_...")
    .build()
    .await?;
```

See [`docs/gitbooks/en/developing/tinyhumans-api-key.md`](../../docs/gitbooks/en/developing/tinyhumans-api-key.md)
for what one TinyHumans API key unlocks across inference, search, embeddings,
media, integrations, voice and Jev.

### A backend request, end to end

When a hosted caller in this crate (for example `BackendClient::authed_json` or
the integrations client) calls the backend, it builds a `BackendRequest`
(`src/transport/port.rs`):

```text
 hosted caller
   |  BackendRequest { method, path, base_url, credential, profile, ... }
   v
 SdkBackendTransport::send_json            (src/transport/mod.rs)
   |  pick reqwest::Client for profile (Api 120 s, Integrations 60 s)
   |  TinyHumansClient::new(base_url)
   |    .with_http_client(..) .with_default_headers(x-sdk-name)
   |    .with_token(jwt)  or  .with_api_key(key)
   v
 tinyhumans_sdk raw().send(..)             (route policy, envelope unwrap)
   |
   +-- Ok(Value) ------------------------------------------> caller
   +-- Err(tinyhumans_sdk::Error) -> map_sdk_error -> BackendTransportError
```

The transport also answers `base_url` (control-plane or inference base, from
`backend::url`), `product_identity`, `attribution_headers` and `http_client`.
The core holds no hosted URL, header policy or product identity of its own; it
reads `Config.resolved_endpoints`, which this crate's enricher
(`client::enricher`, registered with `config::register_enricher`) fills.

## Layout

| Path | What it does |
| --- | --- |
| [`src/lib.rs`](src/lib.rs) | Crate root: module declarations and the public re-exports (`install`, `RuntimeBuilder`, the session types, `SdkBackendTransport`, `ProductIdentity`). Its `embed` module is a curated `pub use` list of the embed items rpc, the hosts and library users take (not the crate); it forwards embed's `#[doc(hidden)] __host` to rpc only. |
| [`src/install.rs`](src/install.rs) | `install`, `is_installed`, `InstallOptions`, `InstallError`. Process-global, idempotent. |
| [`src/runtime.rs`](src/runtime.rs) | `RuntimeBuilder` and `RuntimeError`: an embed builder that connects this layer on `build()`. |
| [`src/backend/`](src/backend/README.md) | Where the backend is (`url.rs`), how requests are attributed (`headers.rs`), and who they are attributed to (`product.rs`). |
| [`src/transport/`](src/transport/README.md) | The `BackendTransport` port types (`port.rs`), `SdkBackendTransport`, the implementation, and `map_sdk_error`. |
| [`src/client/`](src/client/mod.rs) | `BackendClient`, `HostedApiError`, `flatten_authed_error`, `IntegrationsHttp`, the credential endpoint guards and the config enricher. |
| `src/integrations/`, `src/voice/`, `src/inference/`, `src/channels/`, `src/memory/` | The hosted providers, each registered into a core registry by its `register` function. |
| [`src/register.rs`](src/register.rs) | `register_all`: the one place this crate plugs into the core's registries. |
| [`src/hosted/`](src/hosted/README.md) | The hosted-backend RPC proxies: `billing`, `team`, `referral`, `announcements`, `webhooks`, `channel_link`, `oauth`. |
| [`src/session/`](src/session/README.md) | The host-side login and session owner: `SessionManager`, `SessionClient`, `CurrentUserCache`, `CoreLink`. |
| [`src/jev/`](src/jev/README.md) | The Jev-backed `tool_search` ranker (behind the `jev` feature). |
| [`src/jwt.rs`](src/jwt.rs) | Re-exports the SDK's JWT readers (`bearer_authorization_value`, `decode_jwt_exp_unix`, `decode_jwt_payload`) for hosts that already depend on this crate. |

## Key types and entry points

- `install` / `InstallOptions` (`src/install.rs`): connect a core booted by any
  host. Builder methods `product_identity`, `hosted_controllers` and
  `tool_ranker` adjust the defaults.
- `RuntimeBuilder` (`src/runtime.rs`): boot an embed runtime that is already
  connected.
- `SdkBackendTransport` ([`src/transport/mod.rs`](src/transport/mod.rs)): the transport itself, with
  `new()` and `shared()` for hosts that want to install it by hand.
- `hosted_controllers` (re-export of `hosted::extension`): the hosted RPC
  surface as one `ControllerExtension`, for hosts that register it themselves.
- `SessionManager<L: CoreLink>` ([`src/session/manager.rs`](src/session/manager.rs)): what the desktop
  shell and the TUI drive for login, logout and the current user.
- `set_product_identity` / `ProductIdentity` ([`src/backend/product.rs`](src/backend/product.rs)): set the
  `x-sdk-name` a product reports, once, before any backend traffic.

## RPC surface

`install` registers the hosted controllers under `DomainGroup::Hosted`. Wire
names are `openhuman.<namespace>_<function>`:

| Namespace | Methods |
| --- | --- |
| `billing` | `get_summary`, `get_current_plan`, `get_balance`, `purchase_plan`, `create_portal_session`, `top_up`, `create_coinbase_charge`, `get_transactions`, `get_auto_recharge`, `update_auto_recharge`, `get_cards`, `create_setup_intent`, `update_card`, `delete_card`, `redeem_coupon`, `get_coupons` |
| `team` | `get_usage`, `list_members`, `list_teams`, `get_team`, `create_team`, `update_team`, `delete_team`, `switch_team`, `leave_team`, `join_team`, `create_invite`, `remove_member`, `change_member_role`, `list_invites`, `revoke_invite` |
| `referral` | `get_stats`, `claim` |
| `announcements` | `get_latest` |
| `webhooks` | `list_tunnels`, `create_tunnel`, `get_tunnel`, `update_tunnel`, `delete_tunnel`, `get_bandwidth` (shared namespace) |
| `auth` | `create_channel_link_token`, `oauth_connect`, `oauth_list_integrations`, `oauth_fetch_integration_tokens`, `oauth_revoke_integration`, `oauth_fetch_client_key` (shared namespace) |
| `channels` | `telegram_login_start`, `telegram_login_check`, `discord_link_start`, `discord_link_check` (shared namespace) |

The `webhooks`, `auth` and `channels` namespaces also hold core controllers;
the core keeps describing them. See [`src/hosted/README.md`](src/hosted/README.md)
for the per-domain detail.

## Features

The crate forwards every core gate 1:1 to `openhuman-embed`, which forwards it
to the core (`http-server`, `inference`, `voice`, `web3`, `channels`, and the
rest). Its one gate of its own is `jev`, on by default: it compiles in
`tinytools-jev` and `tinyjevclient` and the `jev` module. The desktop shell
enables it explicitly (`features = ["jev"]`) because it builds this crate with
`default-features = false`. Without `jev` the harness ranks `tool_search` with
BM25 alone. [`scripts/ci/check-feature-forwarding.mjs`](../../scripts/ci/check-feature-forwarding.mjs) checks the forwarding
chain core, embed, tinyhumans, cli.

## Boundaries

- This crate owns the port (`BackendTransport`, `BackendRequest`,
  `BackendTransportError` in [`src/transport/port.rs`](src/transport/port.rs)),
  the authenticated JSON client ([`src/client/`](src/client/mod.rs)) and all
  recovery from errors. The core owns only the generic credential type and
  its resolution (`security::credentials::session_support::resolve_backend_credential`),
  which this crate calls. `scripts/ci/check-core-hosted-free.mjs` keeps hosted
  names out of the core.
- Route implementations belong in the vendored SDK (`tinyhumansai/tinyhumans-sdk`,
  [`vendor/tinyhumans-sdk`](../../vendor/tinyhumans-sdk/)). Its unexposed-route registry is the route policy
  the transport enforces. The transport never adds a route the core does not
  already name. Two team routes the SDK does not carry (`POST /teams`,
  `DELETE /teams/{id}`) stay on this crate's `BackendClient::authed_json` until
  the SDK gains them.
- The core never obtains, validates, exchanges or refreshes a credential. It
  takes one through `auth.set_credential`. Login-token exchange, `GET
  /auth/me` and the current-user cache live in `session/` here.
- Hosted memory does not ride this transport. `src/memory/` registers the
  `tinyhumans` engine with `memory::engine_registry`; it hands TinyMemory's own
  HTTP client an endpoint (`backend::url`), the attribution headers
  (`backend::headers`) and a bearer source, but its `/memory/*` requests never
  pass through `SdkBackendTransport` and the SDK route registry does not apply
  to them.
- Realtime Socket.IO stays on the core's own `platform::socket` transport; the
  SDK is built with `default-features = false`, which drops its socket client.
- The generic Jev types (`JevRanker`, `JevEvaluator`, `JevStrategy`) live in
  [`vendor/tinyagents/vendor/tinytools/crates/tinytools-jev`](../../vendor/tinyagents/vendor/tinytools/crates/tinytools-jev/)
  (`tinyhumansai/tinytools`). This crate owns only the wire and credential.

## Gotchas

- Set the product identity before the transport is built. `install` handles
  ordering for you if you pass it in `InstallOptions`; calling
  `set_product_identity` after `install` updates `attribution_headers()` (built
  fresh per call) but not the default headers baked into the already-built
  `reqwest::Client`s.
- `install` must run before the first backend-touching dispatch (it fills the
  core's registries). A suite that boots the core in-process and forgets it sees `BACKEND_UNAVAILABLE:` from
  every backend call. Suites that spawn the `openhuman-core` binary get it from
  `main.rs`.
- `InstallOptions::hosted_controllers(false)` keeps the proxies out of the
  registry entirely. Leaving it on with a `DomainSet` that excludes `hosted`
  also works: the group gate hides them.

## Tests

Tests sit beside their modules as `*_tests.rs` files. `transport_tests.rs` pins
the wire shape the core's classifiers depend on (bearer versus `x-api-key`,
attribution headers, `Status` and `Envelope` mapping, SDK route refusal)
against a wiremock backend, and `channel_404_tests.rs` pins the
channel-message 404 split. The hosted domains test against wiremock with
`hosted/test_support.rs`; `session/*_tests.rs` drive the login owner against
an in-process axum stub backend and a stub `CoreLink` from
`session/test_support.rs`.

```bash
cargo test -p openhuman-tinyhumans
cargo test -p openhuman-tinyhumans session::
pnpm debug rust openhuman_tinyhumans
```

## Further reading

- [`docs/gitbooks/en/developing/tinyhumans-api-key.md`](../../docs/gitbooks/en/developing/tinyhumans-api-key.md): running on a TinyHumans API key.
- [`docs/gitbooks/en/developing/architecture.md`](../../docs/gitbooks/en/developing/architecture.md): architecture overview.
- [`docs/gitbooks/en/developing/embedding.md`](../../docs/gitbooks/en/developing/embedding.md): embedding the core in another product.
- [`crates/README.md`](../README.md): crates overview.
- [`vendor/tinyhumans-sdk/README.md`](../../vendor/tinyhumans-sdk/README.md): tinyhumans-sdk.
- [`src/hosted/announcements/`](src/hosted/announcements/README.md): the `announcements` module README.
- [`src/hosted/billing/`](src/hosted/billing/README.md): the `billing` module README.
- [`src/hosted/channel_link/`](src/hosted/channel_link/README.md): the `channel_link` module README.
- [`src/hosted/oauth/`](src/hosted/oauth/README.md): the `oauth` module README.
- [`src/hosted/referral/`](src/hosted/referral/README.md): the `referral` module README.
- [`src/hosted/team/`](src/hosted/team/README.md): the `team` module README.
- [`src/hosted/webhooks/`](src/hosted/webhooks/README.md): the `webhooks` module README.
