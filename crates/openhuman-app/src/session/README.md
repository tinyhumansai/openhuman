# session

The desktop shell's login glue. Login-token exchange, `GET /auth/me`, the
current-user cache and the credential handoff are implemented in
[`crates/openhuman-tinyhumans`](../../../openhuman-tinyhumans/) (`SessionManager`). This module gives that
manager a link to the core, exposes it to the renderer as Tauri commands, and
forwards its change events to the renderer as Tauri events.

## How it works

The core only holds a credential. It receives one through the
`auth.set_credential` RPC and never talks to the backend's auth endpoints.
Everything that does talk to them runs here, in the shell process.

```text
renderer                 session (shell)                 core
--------                 ---------------                 ----
auth_login_with_token -> SessionHost.manager
                           | POST /auth/login-token/consume  (backend)
                           | GET  /auth/me                   (backend)
                           | HttpCoreLink.invoke ----------> auth.set_credential
                           v
                         SessionEvent::Changed
  <- "auth://changed" ---- event forwarder task
  <- "auth://expired" ---- SessionEvent::Expired { source }
```

`install` runs during Tauri `setup()`, right after the `CoreProcessHandle` is
created. It builds a `SessionHost`, subscribes to the manager's broadcast
channel, registers the host as managed state, and spawns a task that emits
`auth://changed` (payload: `openhuman_tinyhumans::SessionState`) and
`auth://expired` (payload: `{ source }`) to the renderer.

The manager's backend requests carry `ClientHeaders` built from the product
identity, with `CARGO_PKG_VERSION` as both the core and Tauri version, since
the shell and core ship as one release.

`HttpCoreLink` implements `openhuman_tinyhumans::CoreLink` over the same path
the renderer uses. On every call it resolves `(url, token)` through
`crate::active_rpc_endpoint`, so a gateway switch or core restart is picked up
without rewiring. It builds the body with `openhuman_rpc::request_body`, sends
it with `core_rpc::post_json_rpc` (which refuses a bearer over plain HTTP off
loopback), and decodes with `openhuman_rpc::decode_response`.

## Layout

| File | What it does |
| --- | --- |
| [`mod.rs`](mod.rs) | `SessionHost`, `install`, the event names, and `peek_user_id`. |
| [`link.rs`](link.rs) | `HttpCoreLink`, the `CoreLink` over the shell's RPC path. |
| [`commands.rs`](commands.rs) | The `auth_*` Tauri commands, each a one-line delegate to the manager. |

## Tauri commands

| Command | Behavior |
| --- | --- |
| `auth_login_with_token(token)` | Exchanges a one-time login token for a session JWT, validates it, installs it in the core. |
| `auth_store_session(token, user?)` | Installs a JWT (validated against the backend first) or the offline local token the renderer already holds. |
| `auth_logout()` | Clears the credential in the core. |
| `auth_state()` | The core's credential state plus the cached user. |
| `auth_current_user(force?)` | The `/auth/me` user, from cache unless `force` is true. |

All return `Result<_, String>`. The error string is
`openhuman_tinyhumans::SessionError`'s display text, which starts with a
stable `PREFIX:` so the frontend can classify it without parsing prose.

## Key types and entry points

- `SessionHost` ([`mod.rs`](mod.rs)) is managed Tauri state holding
  `Arc<SessionManager<HttpCoreLink>>`.
- `install(app, desktop)` (`mod.rs`) is called once from `lib.rs`.
- `peek_user_id()` (`mod.rs`) returns the signed-in user id synchronously; the
  Sentry `before_send` hook in `lib.rs` uses it to tag events.
- `AUTH_CHANGED_EVENT` and `AUTH_EXPIRED_EVENT` (`mod.rs`) are the event names.

## Boundaries

- Auth endpoints, token validation, caching and identity live in
  [`crates/openhuman-tinyhumans`](../../../openhuman-tinyhumans/) (`openhuman_tinyhumans::session`). Changes to
  login behavior belong there.
- Credential storage and what the core does with a credential (user-dir
  activation, gated services) belong to the core
  (`security::credentials`).
- `get_active_user_id` is a separate command in `lib.rs`, not part of this
  module.

## Tests

[`commands_tests.rs`](commands_tests.rs) covers the command delegates.

```bash
cargo test --manifest-path crates/openhuman-app/Cargo.toml session::
```

## Further reading

- [`docs/gitbooks/en/developing/architecture/tauri-shell.md`](../../../../docs/gitbooks/en/developing/architecture/tauri-shell.md): the Tauri shell.
- [`crates/openhuman-app/README.md`](../../README.md): the openhuman-app crate README.
- [`crates/openhuman-tinyhumans/README.md`](../../../openhuman-tinyhumans/README.md): the openhuman-tinyhumans crate README.
