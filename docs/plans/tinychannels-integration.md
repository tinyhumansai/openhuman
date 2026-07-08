# TinyChannels Integration Plan

Plan for adopting the `tinychannels` crate
(`~/work/tinyhumansai/tinychannels`) as the typed channel layer, deleting the
duplicated copies in this repo, and fixing the channel bugs surfaced by the
2026-07-04 cross-repo audit.

Companion docs in the tinychannels repo:

- `docs/spec/openclaw-hermes-channel-porting.md` — the upstream research spec.
- `docs/spec/tinychannels-execution-plan.md` — the crate-side phased plan.
  Steps below reference its phases.

## Current State

- This repo now depends on `tinychannels` through the TinyChannels
  `feat/phase-0-hygiene` branch while the companion PR lands. The first
  duplicate-removal pass has landed:

  | openhuman-4 file | tinychannels file |
  | --- | --- |
  | `src/openhuman/channels/traits.rs` | re-exports `src/traits.rs` |
  | `src/openhuman/channels/controllers/definitions.rs` | re-exports `src/controllers/definitions.rs` |
  | `src/openhuman/channels/controllers/schemas.rs` schema declarations | converts from `src/controllers/schemas.rs` |
  | `src/openhuman/channels/controllers/ops/connect.rs` allowlist/key helpers | calls `src/controllers/credentials.rs` |
  | `src/openhuman/channels/controllers/ops/types.rs` | re-exports `src/controllers/types.rs` |
  | `src/openhuman/config/schema/channels.rs` | re-exports provider config from `src/config.rs`, while keeping OpenHuman-owned security/sandbox config local |
  | `src/openhuman/channels/context.rs` key/constants helpers | calls/re-exports `src/context.rs` where type-compatible |
  | `src/openhuman/channels/runtime/supervision.rs` in-flight sizing | re-exports `src/runtime.rs::compute_max_in_flight_messages` |
  | Telegram/Discord text splitting | calls `src/text.rs` chunker with UTF-16 measurement |

- `OpenHumanChannelBackend` now lives in
  `src/openhuman/channels/controllers/backend.rs` and implements
  `tinychannels::ChannelBackend` by delegating to the existing
  `channels/controllers/ops/{connect,messaging,discord,telegram}.rs` flows.
- The crate-side Phase 5 relay contract now includes typed gateway/connector
  relay frames, the request/response frame transport loop, a feature-gated
  WebSocket dialer with reconnect supervision, and a portable relay runtime
  config shape under `channels_config.relay`. OpenHuman now enables the
  crate's relay WebSocket feature and starts the relay runtime for complete
  relay config.
- The metadata controllers (`channels.list` / `channels.describe`) now use
  `ChannelManager` for definition lookup. The `channels.status` and
  `channels.test` controllers now route through
  `ChannelManager<OpenHumanChannelBackend>`, preserving the existing no-log
  JSON response shapes while using the crate manager for definition lookup and
  credential validation where applicable. `channels.get_default` also routes
  through the manager while preserving the public `{ active_channel }` response,
  `channels.connect` and `channels.set_default` preserve their public log/value
  envelopes after manager dispatch, and the managed Telegram/Discord link
  controllers route through the manager while preserving their no-log typed
  responses. The no-log send/reaction/thread controllers now dispatch through
  the manager and unwrap the backend `raw` payload to preserve legacy top-level
  JSON shapes; `channels.send_message`, direct channel-bus backend sends, and
  legacy runtime/proactive/provider-helper `Channel::send` callers now cross
  the crate intent bridge as `ChannelOutboundIntent`s and get deterministic
  idempotency keys before legacy backend/provider calls.
  `channels.disconnect` also routes through the manager while preserving the old
  log string and extra fields such as
  `memory_chunks_deleted`. Discord guild/channel/permission discovery dispatches
  through the manager while restoring the old log strings and top-level provider
  JSON. Outbound sends now use the live relay transport for configured relay
  identities. The remaining OpenHuman work is still full session identity
  switchover, provider extraction, and the planned test split below; the first
  envelope slice is landed because the runtime now publishes a TinyChannels
  `ChannelInboundEnvelope` on `ChannelMessageReceived` events, memory
  conversation persistence records the TinyChannels session key as migration
  metadata, and startup now has a relay inbound handler that preserves the
  original authenticated relay envelope, including `scope_id`, from the live
  relay socket through dispatch.

## Step 1 — Add the dependency, delete duplicates

- **Landed:** Add the `tinychannels` git dependency to `Cargo.toml` (single
  crate, not a workspace; edition 2021 depending on an edition-2024 crate is
  fine).
- **Landed for the first slice:** Delete the duplicated
  definitions/types/traits/config-schema/helper code and
  re-export from the old paths so the 100+ call sites keep compiling:
  - `src/openhuman/channels/mod.rs`: `pub use tinychannels::{Channel,
    ChannelMessage, SendMessage, ...};`
  - `src/openhuman/channels/controllers/...`: re-export `ChannelDefinition`,
    `ChannelAuthMode`, response types.
  - `src/openhuman/config/schema/channels.rs`: re-export `ChannelsConfig` and
    provider config structs. Note tinychannels inlined `EmailConfig` and
    `YuanbaoConfig`, which this repo currently sources from
    `channels::email_channel` / `providers::yuanbao` — repoint those two to
    the crate and delete the local definitions.
- Still pending: delete migrated in-module tests once each suite is fully
  represented in tinychannels. The duplicate definitions, config-schema, and
  in-flight sizing helper suites have been deleted; keep everything provider-
  or app-specific (see Step 4).
- Do not remove sandbox/security config from this repo based on the crate's
  copy: the crate is dropping its unwired `SecurityConfig`/`SandboxConfig`
  cluster as scope creep; this repo's originals (if used) stay where they are.

## Step 2 — Implement `ChannelBackend`

- **Landed:** New `OpenHumanChannelBackend` in
  `src/openhuman/channels/controllers/backend.rs`, delegating each trait
  method to the existing ops functions:
  - `send_message` → `messaging.rs::channel_send_message` (already composes
    `effective_backend_api_url` + `jwt::get_session_token` +
    `BackendOAuthClient`), with `send_message_value` preserving OpenHuman's
    arbitrary rich-message JSON for the public RPC controller.
  - `connect_channel` / `disconnect_channel` → `connect.rs` flows plus
    `credentials::ops::{store,remove}_provider_credentials` (keyed
    `"channel:<slug>"`), with disconnect returning the raw legacy payload for
    fields such as `memory_chunks_deleted`.
  - `channel_status` / `test_channel` → existing status/test ops plus
    `health::snapshot`.
  - Telegram login and Discord link/guild/permission methods → the existing
    `telegram.rs` / `discord.rs` ops.
- **Landed:** The current `channels.*` controller entry points dispatch through
  `ChannelManager<OpenHumanChannelBackend>` where they cross this crate seam,
  while preserving the legacy public JSON/log envelopes.
- The event bus, health bus, and dispatch engine stay app-side and *drive*
  tinychannels. Never add a tinychannels → openhuman dependency; the
  `runtime/` dispatch engine and the `web` provider are consumers of the
  crate, not porting candidates (they import ~45 openhuman modules).

## Step 3 — Bug fixes coordinated with the crate

Fix these here in lockstep with tinychannels Phase 1/2 (same semantics, one
implementation — prefer calling the crate's new helpers over patching local
copies):

1. **Landed: UTF-16 chunking.** Telegram and Discord splitting now call the
   crate's Phase 2 chunker with UTF-16 code-unit measurement and markdown fence
   preservation.
2. **Landed for existing helper semantics: Telegram history keys.**
   `channels/context.rs` now delegates to the crate helper, preserving
   OpenHuman's current Telegram topic behavior. Runtime received-message events
   now also carry the crate's normalized inbound envelope, with Telegram
   `thread_ts` projected as `topic_id`; conversation persistence records the
   TinyChannels session key when an event carries an envelope. Relay inbound now
   preserves the relay-supplied envelope and `scope_id` through dispatch. Full
   non-relay session identity switchover still needs provider-sourced
   `scope_id`.
3. **Partially landed for relay inbound; non-relay providers still deferred:
   workspace/tenant discriminator.** Session keys
   (`channels/bus.rs:1005-1042`) omit guild/team/tenant; Slack channel ids are
   only workspace-unique. Relay inbound carries the relay envelope's `scope_id`
   through received-message events; provider-originated legacy messages still
   need native envelope construction with provider-sourced scope. Deferral
   reason: adding scope to the current legacy string keys would change
   conversation identity without the envelope migration that can preserve
   compatibility aliases.
4. **Landed for portable send boundaries; provider wire enforcement remains
   platform-specific.**
   No idempotency key exists on legacy sends; a retry after a transport error
   double-posts. `channels.send_message` now routes through a crate
   `ChannelOutboundIntent` at the `ChannelBackend` seam while preserving the
   legacy public RPC response and backend JSON shape. The direct channel bus
   backend sends and runtime/proactive/provider-helper `Channel::send` paths use
   the same crate bridge, carrying deterministic idempotency keys through
   `SendMessage`. Provider-specific use of those keys on external wire APIs
   remains Phase 6/platform work.
5. **Deferred pending product decision: `conversation_memory_key` intent
   check.** It keys on `msg.id`
   (per-message, not per-conversation) and this repo's tests assert that
   behavior (`tests/memory.rs`). Confirm whether per-turn keying is intended;
   the crate will rename or fix accordingly — this repo should follow.

## Step 4 — Test split

- **Partly landed:** the portable `controllers/schemas_tests.rs` schema-catalog
  assertions, pure allowlist/key helper tests, catalog/request-shape halves of
  `controllers/ops_tests.rs`, backend-agnostic default-channel validation, and
  config-backed channel status detection now live in tinychannels. The
  duplicate definitions suite, config-schema assertions, in-flight sizing
  helper assertions, and portable schema-catalog assertions were deleted here.
  This repo keeps handler parity, adapter conversion, params, legacy envelope
  helper tests, local security/sandbox config defaults, supervisor
  classification tests, and app-side persistence/REST wiring tests.
- **Still migrate to tinychannels:** any remaining backend-agnostic assertions
  from `controllers/ops_tests.rs`, rewritten against a mock `ChannelBackend`,
  plus the already-mirrored suites listed in Step 1.
- **Stay here:** all `providers/*_tests.rs` (telegram 148, web 93, imessage
  58, discord 55+23, email 50, lark 42, whatsapp 41+29, irc 37, mattermost
  33, signal 32, presentation 29, linq 23, qq 11), `bus_tests.rs`,
  `routes_tests.rs`, `runtime/dispatch_tests.rs`, `runtime/startup_tests.rs`,
  and the harness-driven `tests/` integration files (prompt, telegram/discord
  integration, runtime tool calls, health, identity).
- The REST-wiring halves of `ops_tests.rs` become the test bed for
  `OpenHumanChannelBackend` (Step 2).

## Step 5 — Provider extraction ladder (later, tracks crate Phase 6)

Move provider wire code into tinychannels only when its dependencies reduce to
crate traits:

1. **Now portable (no cross-module imports):** email, irc, yuanbao, cli,
   imessage, mattermost, qq, dingtalk, presentation.
2. **After a configured-HTTP-client trait** (replaces
   `config::build_runtime_proxy_client` / `apply_runtime_proxy_to_builder`):
   discord, slack, whatsapp, lark, signal.
3. **After approval, voice/STT, pairing, and conversation-memory traits:**
   telegram (imports `approval`, `voice::create_stt_provider`,
   `memory_conversations`, `security::pairing`).
4. **Never:** `web` provider and `runtime/` dispatch — they are the harness
   front-end and orchestrator that consume the crate.

## Sequencing and risk notes

- Land Step 1 and Step 2 together in one PR if possible: adding the dep while
  keeping duplicates invites split-brain imports (some call sites on the
  crate types, some on the local copies — Rust will treat them as different
  types).
- Session-key changes (Step 3, items 2–3) change conversation identity. Ship
  them behind the crate's legacy-key canonicalization with a migration test
  that maps a sample of existing keys to new keys losslessly.
- Positive invariants to preserve (verified in the audit): no secrets are
  logged in the channels tree, and no blocking calls exist in async provider
  paths. Any moved code must keep both properties.
