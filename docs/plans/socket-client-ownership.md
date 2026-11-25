# Socket.IO client ownership design

Status: design for review, 2026-10-19  
Scope: `tinyhumansai/openhuman#8089`, design leaf `senamakel/openhuman#242`

## Decision

Choose option (a): keep the OpenHuman-facing socket manager and a generic
connection port in `openhuman-core`, and register the TinyHumans implementation
from `openhuman-tinyhumans::register_all`. The implementation uses
`tinyhumans-sdk::SocketConnection`. Keep the core free of a dependency on
`tinyhumans-sdk`.

The repository's Backend API boundary says that only `openhuman-tinyhumans`
may depend on `tinyhumans-sdk` and that hosted behavior enters the core through
generic registries (`CLAUDE.md`, “Backend API”). The SDK already implements the
Socket.IO protocol connection, receives named JSON/binary events, emits events,
and handles acknowledgements (`vendor/tinyhumans-sdk/src/socket/mod.rs`). The
core's `ws_loop` separately implements Engine.IO and Socket.IO handshakes,
frames, acknowledgements, redirects, and heartbeats
(`crates/openhuman-core/src/platform/socket/ws_loop/`). Reusing the SDK behind a
core port removes that second protocol client without crossing the hosted-free
dependency boundary.

Option (b) would require a new protocol crate shared by the SDK and core. That
duplicates an SDK transport that is already present and would split the
Socket.IO contract across a third package. No current source or test shows a
need for a second generic protocol implementation.

## Ownership after the move

| Owner | Owns | Must not own |
| --- | --- | --- |
| `tinyhumans-sdk` | Socket.IO and Engine.IO framing, namespace connect acknowledgement and rejection, server-assigned namespace id, JSON/binary payload conversion, acknowledgement wire behavior, close/error events, bounded one-attempt connect, redirect mechanics, and cancellation-safe ownership/termination of every SDK-created polling or reconnect task. | OpenHuman status enums, its retry schedule, session-store reads, Sentry policy, or event-bus mapping. |
| `openhuman-tinyhumans` | The SDK adapter and registration in `register_all`; translates the SDK connection into the core's generic port contract. It enables the SDK `socket` feature explicitly because its Cargo dependency currently disables SDK defaults and therefore omits the socket module (`crates/openhuman-tinyhumans/Cargo.toml`). | OpenHuman's RPC schemas, `DomainEvent` types, or policy decisions about user-visible connection status. |
| `openhuman-core` | The generic port contract and registry, `SocketManager` facade, consumer-facing RPC and event-bus routing, live credential-provider callback, lifecycle/reconnect policy, queue/ack waiter rules, status and diagnostics, and the existing consumer API. The connection driver calls only the registered port and knows no TinyHumans SDK types or hosted strings. The port has an awaited, bounded close contract; account rebind waits for the old SDK session to terminate before opening the new credential identity. | Socket.IO packet parsing, Engine.IO ping/pong, TinyHumans hostnames/routes, SDK imports, or an SDK workaround copied into core. |

The port is a connector plus a connected-session handle. The connector accepts
the resolved endpoint and a credential for one attempt, and returns a handle
that reports connected, rejected, transport-failed, and closed outcomes; the
handle receives generic named payloads and supports emit, emit-with-ack, and
intentional close. `close` is awaited and bounded, and the port contract
guarantees that it does not return until the adapter's SDK session and every
SDK-owned poll/reconnect task for that identity have stopped. Connector
attempts are cancellation-safe too: dropping a timed-out attempt cannot leave a
background task that authenticates later. Account rebind must finish closing
the old handle before a new credential identity is connected. Keep the
interface independent of `SocketConnection` types. Store it in the same
generic extension/registry mechanism used by the other hosted providers,
including the per-`CoreContext` path. A host with no socket connector retains
today's hosted-free behavior: no auto-connect and a stable unavailable result
for calls that require the hosted connection.

`openhuman-tinyhumans::register_all` adds one idempotent socket registration.
The public registry shape remains in core; the SDK adapter lives under
`crates/openhuman-tinyhumans/src/socket/`. Do not add SDK imports,
dependencies, or re-exports to core or `openhuman-embed`. Only SDK-independent
core port registration types may cross `openhuman-embed::__host` for use by
`openhuman-tinyhumans`.

## Lifecycle, credentials, endpoint, and redirect constraints

- Keep the retry supervisor in the OpenHuman host-facing layer. It reads a
  fresh credential before each attempt through the existing token-provider
  seam. An empty/missing credential stops the loop and surfaces the existing
  “sign in again” state. A server credential rejection gets one fresh-provider
  check; retry immediately only when the credential changed, and bound that
  immediate retry to one per failure cycle. Otherwise stop instead of burning
  the ordinary backoff. Keep the current 1 s exponential backoff capped at 30
  s, status transition to `Reconnecting`, one-shot fifth-failure observability
  escalation, and explicit loop-liveness reporting.
- Keep `connect(url, token)` as the static-credential entry point and
  `connect_with_session` as the live-provider entry point. The backend session
  credential remains the credential supplied through OpenHuman's existing
  credential seam; the TinyHumans adapter passes it to the SDK for the
  Socket.IO namespace auth object. The core still does not exchange, refresh,
  or validate credentials. Preserve the local/offline-token refusal and never
  log token contents.
- `Config.resolved_endpoints.socket_url` already exists and the TinyHumans
  enricher fills it as a Socket.IO endpoint
  (`crates/openhuman-core/src/config/schema/resolved_endpoints.rs`,
  `crates/openhuman-tinyhumans/src/client/enricher.rs`). The current connect
  RPC and boot auto-connect instead read `control_plane_url` and derive the
  WebSocket path with `websocket_url` (`platform/socket/ops.rs`,
  `core/runtime/services.rs`). The port migration should consume the resolved
  socket endpoint; keep `config.api_url` override behavior in the enricher.
  The current SDK `TinyHumansClient::connect_socket()` starts from its HTTP
  client's base URL and lets `rust_socketio` add `/socket.io/`; the resolved
  `socket_url` already contains `/socket.io/?EIO=4&transport=websocket`. Add
  an SDK entry point whose input is explicitly that ready-to-dial endpoint, or
  have the adapter construct the SDK client from the control-plane origin and
  prove the configured resolved endpoint is equivalent. The preferred design
  is the explicit endpoint entry point so the core does not reconstruct hosted
  URL rules.
- Each SDK connect hop must have the current ten-second deadline covering DNS,
  TCP, TLS, and the WebSocket upgrade. Bound each redirect hop independently
  and cap redirects at three. Follow 301, 302, 307, and 308 only; resolve
  relative `Location` values against the current URL. Keep the current
  TLS-or-loopback endpoint guard for every hop and reject downgrade redirects
  to remote plaintext because the namespace auth contains a bearer credential.
- Return redirect metadata (status, original endpoint, final endpoint) to the
  host. The host pins the final endpoint for reconnects and warns once when a
  permanent 301/308 makes the configured endpoint stale. Temporary 302/307
  redirects do not produce that warning. The SDK performs the redirect; it
  does not decide how OpenHuman displays it.
- SDK internal reconnection must be disabled for the OpenHuman adapter. The
  dependency `rust_socketio 0.6.0` defaults its `ClientBuilder` to
  `reconnect = true` and retries internally with its own backoff; the wrapper
  currently does not override that default. Running both supervisors would
  bypass OpenHuman's fresh-credential, terminal-auth, status, diagnostics, and
  shutdown rules. The SDK therefore needs a tested caller-selected
  single-attempt mode. The existing SDK convenience method may retain its
  reconnecting default for other SDK callers.
- Cancellation and task ownership are also SDK requirements. In the locked
  `rust_socketio 0.6.0`, `Client::poll_stream` spawns a detached task containing
  a cloned client. The SDK wrapper retains no join handle, and
  `SocketConnection::disconnect()` delegates to `Client::disconnect()` without
  joining that task. Therefore, aborting OpenHuman's supervisor does not prove
  the old credential-bearing SDK session has stopped. The SDK must make an
  in-flight connect cancellation-safe and provide an explicit, bounded close
  or abort-and-join operation that terminates every SDK-owned poll/reconnect
  task. The generic core port must await that guarantee before shutdown,
  timeout/retry, or identity rebind completes. Do not claim that aborting the
  core task cleans up the SDK's detached task.
- A successful Engine.IO WebSocket upgrade is not sufficient for OpenHuman to
  report `Connected`: the core currently waits for the Socket.IO namespace
  connect acknowledgement and treats its connect-error packet separately.
  The SDK port contract must expose namespace-ready and namespace-rejected
  outcomes, plus the server-assigned namespace id, before the host accepts
  emits. `SocketConnection` currently has no id accessor, while
  `SocketState.socket_id` is public RPC state. This status boundary also
  prevents a silent server from leaving the manager permanently `Connecting`.
- The SDK owns generic Engine.IO heartbeat behavior. The host observes its
  terminal close/error outcome and applies its reconnect policy. Preserve the
  user-visible status, failure classifier, and useful outage duration/count
  diagnostics; do not keep a second core ping/pong timer over the SDK client.
  Exact diagnostics based on raw Engine.IO ping counts are implementation
  details and may be retired if the SDK does not expose equivalent generic
  telemetry. That is the only identified product-diagnostic gap; it must be
  called out in the host issue and reviewed explicitly.

## Consumer and endpoint evidence

The connection is consumed by core-owned product behavior, so the facade and
event routing remain in core:

| Consumer | Socket behavior to retain |
| --- | --- |
| `core/runtime/bootstrap.rs`, `core/runtime/services.rs` | Initialize the manager and auto-connect only when `ServiceSet::socketio` is on and a resolved endpoint plus stored session exist. |
| `platform/socket/schemas.rs`, `core/all.rs` | Keep the `socket` RPC namespace and controller registration. |
| `platform/connectivity/rpc.rs` | Keep connected state, last error, active retry-loop, and terminal-failure diagnostics. |
| `credentials/ops/credential.rs` | Reuse the live connection for credential-related notifications. |
| `security/devices/tunnel_client.rs` | Emit device tunnel registration, frames, and peer operations. |
| `skills/webhooks/{ops.rs,bus.rs}` | Route incoming webhook requests and emit correlated responses. |
| `voice/realtime_harness/chat_delivery.rs` | Deliver live voice harness events through the shared manager. |
| `platform/socket/event_handlers.rs` | Translate accepted server event names into status changes and the existing generic `DomainEvent` bus messages for webhook, Composio, device-tunnel, and channel consumers. Keep payload redaction. |

The renderer listens for the stable `runtime:socket-state-changed` and
`server:event` names. Preserve these wire/event-bridge strings; the existing
`types_tests.rs` pins them.

## Test ownership and migration map

Every existing assertion is assigned below. The SDK tests use a local Socket.IO
fixture only; they must not call the production backend. Core port tests use a
fake registered connector for lifecycle and event routing, then retain one
TinyHumans adapter smoke test through the SDK.

| Existing tests | New owner and disposition |
| --- | --- |
| `ws_loop_dispatch_and_redirect_tests.rs`: `resolve_redirect_upgrades_http_to_ws_for_absolute_location`, `resolve_redirect_handles_relative_location_against_current_url`, `resolve_redirect_preserves_ws_and_wss_schemes_verbatim`, `resolve_redirect_rejects_unsupported_scheme`, `redirect_status_matches_only_followable_codes`, `extract_location_header_returns_value_when_present`, `extract_location_header_returns_none_when_missing`, `connect_with_redirects_fails_when_location_missing` (in `ws_loop_reconnect_tests.rs`) | Move to SDK redirect unit/fixture tests. Add a fixture test for bounded hop count, each supported status, relative resolution, scheme safety, and permanent-vs-temporary metadata. |
| `ws_loop_reconnect_tests.rs`: `ws_loop_follows_301_to_working_backend` | Split: SDK fixture proves the redirected connection reaches namespace-ready; core port test proves 301/308 warning is one-shot, 302/307 is quiet, and reconnect uses the final endpoint. |
| `ws_loop_deadline_tests.rs`: `connect_with_redirects_times_out_when_upgrade_never_answers`, `connect_timeout_is_not_tighter_than_a_handshake_read` | Move to SDK local-fixture tests for a silent upgrade and the configured deadline. Include a non-responsive namespace handshake case because the SDK must report namespace-ready/rejected, not just WebSocket upgrade. |
| SDK-owned task lifecycle (new SDK tests) | Add local-fixture tests that block the upgrade/namespace handshake and close path, cancel an in-flight old-credential connection or rebind to a new identity, then release the old peer. Assert the canceled connection does not authenticate later, emit, deliver events, reconnect, or continue polling. Assert bounded close/abort joins every SDK-owned task. |
| `ws_loop_dispatch_and_redirect_tests.rs`: `handle_eio_message_ping_sends_pong`, `handle_eio_message_pong_is_ignored`, `handle_eio_message_empty_is_noop`, `handle_eio_message_message_routes_to_sio_packet`, `handle_eio_message_close_and_noop_do_not_panic`, `handle_eio_message_unknown_packet_is_utf8_safe_at_preview_boundary` | Move protocol assertions to SDK integration tests for heartbeat, close, and named event delivery. Delete raw packet/log-preview cases after that contract is covered; there is no OpenHuman EIO parser afterward. |
| `ws_loop_dispatch_and_redirect_tests.rs`: `parse_sio_ack_returns_id_and_single_payload_value`, `handle_sio_packet_ack_resolves_pending_ack`, `handle_sio_packet_connect_reack_updates_sid`, `handle_sio_packet_connect_reack_missing_sid_is_noop`, `handle_sio_packet_disconnect_flips_status_and_clears_sid`, `handle_sio_packet_connect_error_does_not_panic`, `handle_sio_packet_empty_is_noop`, `handle_sio_packet_unknown_type_is_noop`, `handle_sio_packet_unknown_type_is_utf8_safe_at_preview_boundary`, `handle_sio_packet_unparseable_event_is_utf8_safe_at_preview_boundary`, `handle_sio_packet_event_with_unparseable_payload_is_logged_only` | Move ACK/namespace/close/wire error behavior to SDK local-fixture tests. Delete tests whose sole assertion is behavior of the removed raw parser or its log truncation. Keep host assertions for how typed outcomes affect manager state. |
| `ws_loop_dispatch_and_redirect_tests.rs`: `handle_sio_packet_event_dispatches_to_event_handler` | Split: SDK proves that a named event and its payload arrive intact; core event-handler test continues to prove that `ready` updates status and the supported domain event names publish the right `DomainEvent`. |
| `ws_loop_dispatch_and_redirect_tests.rs`: `ws_loop_completes_handshake_and_shuts_down_cleanly`, `ws_loop_handles_connect_error_and_shutdown`, `ws_loop_handles_bad_eio_open_and_shutdown`, `ws_loop_refuses_to_start_with_empty_token`, `ws_loop_exits_cleanly_when_provider_returns_error` | Split protocol handshake and server rejection into SDK tests. Replace the core tests with port-level tests for `Connecting` → `Connected`, server rejection → `Error`/terminal policy, bad-connection outcome → retry, shutdown → `Disconnected`, and no attempt when the credential provider is empty or errors. |
| `ws_loop_dispatch_and_redirect_tests.rs`: `connect_behavior_variants_are_distinct` | Delete with the mock enum; it only makes a fixture variant appear used. |
| `ws_loop_deadline_tests.rs`: `ws_loop_reports_reconnecting_between_attempts`, `ws_loop_marks_a_terminal_stop_as_failure` | Keep as core port tests using a deterministic fake connector. These pin the product status contract and the `connectivity_diag` semantics. |
| `ws_loop_deadline_tests.rs`: `ws_loop_reconnects_after_ping_deadline_on_a_silent_server` | Split: SDK proves heartbeat timeout produces a terminal closed/error outcome; core fake-port test proves that outcome enters `Reconnecting` and retries. Retire assertions tied only to raw ping counts unless equivalent SDK diagnostics are adopted. |
| `ws_loop_reconnect_tests.rs`: `ws_loop_calls_provider_before_each_attempt`, `ws_loop_escalates_immediately_on_invalid_token_no_refresh`, `ws_loop_retries_with_fresh_token_on_invalid_token`, `ws_loop_bounds_fresh_token_retries_with_rotating_provider`, `decide_after_invalid_token_fresh_token_returns_retry`, `decide_after_invalid_token_same_token_escalates`, `decide_after_invalid_token_provider_error_escalates`, `decide_after_invalid_token_empty_token_escalates` | Keep in core port/supervisor tests. Inject a fake connector that returns structured credential rejection; assert fresh provider reads, exactly one immediate changed-token retry, no retry for unchanged/missing credentials, and no SDK retry behind the supervisor. |
| `ws_loop_reconnect_tests.rs`: `sio_connect_error_invalid_token_classifies_correctly` | Split: SDK verifies it preserves a namespace rejection as a structured error; core test verifies the generic credential-rejected outcome follows terminal/fresh-token policy. Do not match SDK display text in core. |
| `ws_loop_reconnect_tests.rs`: `draining_reports_and_discards_everything_still_queued`, `draining_an_empty_channel_reports_nothing_dropped` | Keep as core manager queue/teardown tests; ensure queued emits from a dead connection are never replayed on a new session. |
| `ws_loop_reconnect_tests.rs`: `attempt_fraction_parser_reads_the_reported_bug_shape`, `attempt_line_never_reports_more_attempts_than_its_maximum`, `attempt_line_past_the_threshold_drops_the_denominator_for_the_outage`, `sustained_outage_escalates_exactly_once_across_a_long_streak`, `offline_user_outage_does_not_arm_the_recovery_report`, `reproduces_the_6417_outage_without_either_reported_symptom` | Keep as core observability/reconnect-policy tests. |
| `ws_loop_dispatch_and_redirect_tests.rs`: `redirect_warning_is_recorded_once_and_pinned_to_first_hop`, `log_connection_failure_does_not_panic_below_threshold`, `log_connection_failure_does_not_panic_at_and_above_threshold`, `fail_escalate_threshold_is_five`, `sustained_outage_for_network_unreachable_classifies_as_expected`, `sustained_outage_for_tls_handshake_eof_classifies_as_expected`, `sustained_outage_for_actionable_server_error_does_not_classify` | Keep in core host-policy tests, with redirect outcome supplied by the port and the same expected-error classifier inputs. |
| `event_handlers_tests.rs`: `parse_sio_event_accepts_bare_array`, `parse_sio_event_strips_ack_id_prefix`, `parse_sio_event_defaults_data_to_null_when_missing`, `parse_sio_event_returns_none_for_garbage`, `parse_sio_event_returns_none_when_first_element_is_not_string`, `parse_sio_event_returns_none_when_json_invalid` | Move payload argument conversion tests to SDK. Delete the OpenHuman raw-string parser. |
| `event_handlers_tests.rs`: `base64_encode_round_trips_ascii`, `base64_encode_handles_empty_string`, `base64_encode_handles_json_body`, `handle_sio_event_ready_sets_connected`, `handle_sio_event_error_sets_error_status`, `handle_sio_event_debug_truncation_respects_utf8_boundary`, `handle_sio_event_unknown_event_is_noop_on_status`, `handle_sio_event_channel_message_missing_channel_is_dropped`, `handle_sio_event_channel_message_empty_text_is_dropped`, `handle_sio_event_payload_redacted_no_panic_on_multibyte_boundary` | Keep in core. These cover webhook response payload encoding, status/event policy, channel validation, and safe/redacted application logging, independent of the protocol library. |
| `manager_tests.rs`: `connection_rejects_plaintext_remote_socket_before_starting_loop`, `new_manager_is_disconnected_with_no_sid`, `default_impl_matches_new`, `is_connected_tracks_status_transitions`, `get_state_reflects_stored_sid_and_status`, `get_state_surfaces_stored_error_to_callers`, `emit_without_connection_errors_without_panic`, `emit_while_connecting_errors_even_with_emit_channel_present`, `emit_when_connected_queues_the_encoded_event`, `emit_does_not_land_on_a_reconnects_pre_handshake_channel`, `emit_still_succeeds_after_server_error_on_live_connection`, `emit_cannot_send_past_a_concurrent_teardown_drain`, `emit_with_ack_without_connection_errors_without_waiting`, `emit_with_ack_uses_emit_queue_while_connecting`, `disconnect_on_fresh_manager_is_idempotent`, `a_timed_out_socket_loop_is_aborted_and_joined`, `identity_rebind_transactions_are_serialized`, `emit_state_change_is_safe_to_call_on_empty_shared`, `emit_server_event_is_safe_without_subscribers`, `set_webhook_router_populates_the_shared_slot`, `set_webhook_router_overwrites_previous_router`, `emit_after_disconnect_errors_not_connected`, `connect_rejects_empty_token_and_returns_err` | Keep in core as generic `SocketManager`/port tests. Preserve no-plaintext-remote rejection, disconnected/connected state, emit readiness and teardown serialization, ACK timeout/cancellation, idempotent stop, manager supervisor join deadline, identity rebind, event bridge safety, webhook-router attachment, and empty-token validation. In particular, keep `a_timed_out_socket_loop_is_aborted_and_joined` as a host-supervisor guarantee; it is distinct from the SDK task termination tests above. Replace test-only assumptions about encoded SIO wire strings with fake-port calls. |
| `ops_tests.rs`: `a_redundant_session_connect_reuses_the_live_socket`, `a_token_refreshed_mid_loop_updates_the_recorded_identity`, `a_session_connect_with_a_different_token_still_rebinds`, `is_live_for_is_scoped_to_url_and_token_and_cleared_on_disconnect`, `a_session_connect_during_the_boot_handshake_does_not_restart_it`, `a_connecting_status_with_no_live_loop_is_not_treated_as_serving` | Keep in core port tests with a fake connector that can pause before ready and count attempts. Preserve identity serialization and no duplicate live connection. |
| `url_tests.rs`, `token_provider_tests.rs`, `types_tests.rs`, `schemas_tests.rs`, `platform/connectivity/rpc_tests.rs`, `services_tests.rs` | Keep and update only where the port changes. These pin endpoint safety/resolution, credential callback semantics, stable renderer event names, controller/RPC contract, connectivity diagnostics, and `ServiceSet::socketio` boot policy. |

The existing `ws_loop_tests.rs` file is a raw Engine.IO fixture toolkit shared by
the split suites. Delete it after each SDK wire fixture and core fake-port
fixture is in its owning test target. Delete `ws_loop.rs`,
`ws_loop/{connect,dispatch,reconnect}.rs`, and raw protocol types only after
every mapped assertion has moved or been explicitly retired above. Do not
delete event handlers, schemas, manager consumers, or product-policy tests.

## Filed follow-up issues

These issues were filed and linked under `tinyhumansai/openhuman#8089`. The SDK
contract issue is [senamakel/sdk#2](https://github.com/senamakel/sdk/issues/2);
the dependent OpenHuman adapter issue is
[senamakel/openhuman#420](https://github.com/senamakel/openhuman/issues/420).
The SDK issue must land upstream first; its implementation PR must target the
canonical `tinyhumansai/sdk` repository. OpenHuman can then update its SDK
gitlink and implement the adapter/port move in a separate leaf.

### Filed SDK issue: make SocketConnection suitable for a supervised host

**Title:** `socket: expose a bounded single-attempt connection contract`

**Body:**

```markdown
Parent: tinyhumansai/openhuman#8089 (OpenHuman host integration depends on this SDK change)

## Context

`src/socket/mod.rs` exposes `SocketConnection` over `rust_socketio` and
`TinyHumansClient::connect_socket()` connects using the client's current token.
The wrapper does not expose connection options, redirect metadata, a namespace
ready/rejected outcome, or a caller-selected single-attempt mode. The upstream
`rust_socketio 0.6` builder defaults to internal automatic reconnect, which
uses the original auth value unless the wrapper wires its reconnect callback.
OpenHuman's host needs to own its reconnect loop so it can re-read its session
credential before every attempt and stop immediately when the credential is
known to be stale.

## Scope

- Add an explicit socket connection options/API that lets a caller disable
  SDK-internal reconnect and supervise one connection attempt.
- Add an explicit-endpoint connect entry point: it accepts the already-resolved
  Socket.IO endpoint (`/socket.io/?EIO=4&transport=websocket`) without
  appending another path or silently deriving a different host URL.
- Bound DNS, TCP, TLS, WebSocket upgrade, and namespace-connect handshake
  waits; make the per-attempt deadline explicit and test a silent peer.
- Make connect cancellation-safe and connection close bounded. Retain and
  terminate every SDK-owned polling/reconnect task: canceling an in-flight
  connect or closing a connected handle must not leave a detached client that
  authenticates, reconnects, emits, delivers events, or polls after cancellation.
- Follow at most three 301/302/307/308 redirects, including relative
  `Location` values. Return redirect status and original/final endpoint metadata
  to the caller. Reject unsupported schemes and remote plaintext downgrade
  redirects for credential-bearing connections.
- Expose namespace-ready and namespace-rejected outcomes separately from the
  Engine.IO WebSocket-open event, with the rejection payload preserved as
  structured data, and expose the server-assigned namespace id. Preserve
  generic named JSON/binary events and ACK behavior.
- Keep protocol/reconnect defaults for existing SDK callers compatible; the
  OpenHuman caller must be able to select single-attempt mode explicitly.

## Tests

Use a local Socket.IO fixture. Cover ready and rejected namespace handshakes,
silence at the upgrade and namespace handshake stages, redirects and redirect
metadata (including the hop cap and relative location), single-attempt close
without hidden reconnect, cancellation during a blocked connect, bounded close
when the peer stalls, and rebind/cancel followed by releasing the old peer with
assertions that no stale authentication/emits/events/reconnect/polling occur.
Also cover event payload delivery and emit-with-ack. No test calls the
production backend.

## Ownership

The SDK owns generic Socket.IO/Engine.IO behavior. OpenHuman's status, retry,
credential refresh, observability, and event-bus policies stay in its host
adapter/core port.
```

### Filed OpenHuman issue: move socket implementation behind the hosted adapter

**Title:** `platform/socket: replace the hand-rolled client with the TinyHumans SDK adapter`

**Body:**

```markdown
Parent: tinyhumansai/openhuman#8089
Depends on: [senamakel/sdk#2](https://github.com/senamakel/sdk/issues/2),
the single-attempt SocketConnection contract being available upstream and
pinned by the TinyHumans SDK submodule.

## Context

`crates/openhuman-core/src/platform/socket/ws_loop.rs` and `ws_loop/` duplicate
Engine.IO/Socket.IO protocol work already owned by
`vendor/tinyhumans-sdk/src/socket/`. The core must remain free of a
`tinyhumans-sdk` dependency (`CLAUDE.md`, “Backend API”; enforced by
`check-core-hosted-free.mjs`). The design in
`docs/plans/socket-client-ownership.md` keeps the core's consumer-facing manager
and a generic socket connector port, then registers the SDK-backed connector
from `openhuman-tinyhumans::register_all`.

## Scope

- Add the generic connection-port registry/API in core and register the
  TinyHumans adapter through `openhuman-tinyhumans::register_all`.
- Enable the SDK socket feature from `openhuman-tinyhumans` without adding an
  SDK dependency or hosted implementation to core.
- Route manager operations through the port. Keep the `socket` RPC namespace,
  status/event renderer names, event-bus mappings, redacted logging, tunnel,
  webhook, credential, and voice consumers.
- Use `Config.resolved_endpoints.socket_url` from the TinyHumans config
  enricher. Preserve `connect`'s static credential mode, the live session
  provider, per-attempt credential refresh, invalid-token fast-fail and bounded
  immediate refresh retry, current exponential backoff, loop liveness and
  one-shot outage reporting.
- Consume SDK redirect metadata to pin the resolved endpoint and preserve the
  one-shot warning for permanent redirects. Keep the ten-second per-hop
  deadline and three-hop cap. Keep TLS/loopback endpoint safety checks.
- Move protocol tests to the SDK and move host/port policy tests to core's
  generic fake-port tests as mapped in the design doc. Delete the handwritten
  frame parser, direct WebSocket protocol loop, and associated fixture helpers
  only after replacements pass.
- Preserve `a_timed_out_socket_loop_is_aborted_and_joined` as the core
  supervisor's own abort/join guarantee, separately from the SDK's guarantee
  that its detached polling/reconnect work has terminated. Account rebind must
  await bounded SDK close before connecting with the new identity.
- Explicitly record this approved diagnostic tradeoff in the PR body: because
  Engine.IO heartbeat handling moves into the SDK, the core's raw ping-count
  diagnostic may be retired if the SDK exposes no equivalent generic
  telemetry. Preserve connection age, outage duration, failed-attempt count,
  and one-shot outage reporting so operators retain useful drop diagnostics.
- Keep one SDK adapter integration test against a local fixture. Do not add
  external service calls or a host-side protocol workaround.

## Verification

- `cargo tree -p openhuman -i tinyhumans-sdk` remains empty.
- `node scripts/ci/check-core-hosted-free.mjs` passes.
- `pnpm docs:check`, Rust layout, and the affected Rust test targets pass.
- Existing consumers still pass fake-port tests for state, dispatch, emits,
  ACK cancellation, shutdown and endpoint identity.
```

## Evidence index

- Hosted-free and registration rule: `CLAUDE.md`, “Backend API”; `crates/openhuman-tinyhumans/src/register.rs`; `crates/openhuman-tinyhumans/Cargo.toml`.
- SDK public API and current tests: `vendor/tinyhumans-sdk/src/lib.rs`,
  `vendor/tinyhumans-sdk/src/socket/mod.rs`,
  `vendor/tinyhumans-sdk/tests/socket.rs`.
- Endpoint source and existing core usage:
  `crates/openhuman-core/src/config/schema/resolved_endpoints.rs`,
  `crates/openhuman-tinyhumans/src/client/enricher.rs`,
  `crates/openhuman-core/src/platform/socket/ops.rs`,
  `crates/openhuman-core/src/core/runtime/services.rs`.
- Port consumers and stable event names:
  `crates/openhuman-core/src/platform/socket/README.md`,
  `crates/openhuman-core/src/platform/README.md`,
  `crates/openhuman-core/src/platform/socket/types.rs`.
- Existing behavior and tests: `crates/openhuman-core/src/platform/socket/README.md`,
  `ws_loop/connect.rs`, `ws_loop/dispatch.rs`, `ws_loop/reconnect.rs`,
  `ws_loop_deadline_tests.rs`, `ws_loop_dispatch_and_redirect_tests.rs`,
  `ws_loop_reconnect_tests.rs`, `ws_loop_tests.rs`, `event_handlers_tests.rs`,
  `manager_tests.rs`, `ops_tests.rs`, `token_provider_tests.rs`.
- SDK dependency reconnect defaults verified against the locked
  `rust_socketio 0.6.0` source: `asynchronous/client/builder.rs` defaults
  `reconnect` to `true`; `asynchronous/client/client.rs::poll_stream` owns its
  reconnect/backoff task; `asynchronous/socket.rs::handle_socketio_packet`
  observes namespace connect/reject packets. SDK wrapper currently exposes no
  corresponding option or ready/rejected stream event.
