//! The process-wide bus singleton.
//!
//! Everything that used to be `core::event_bus` now comes from `tinybus`. What
//! is left here is the one thing a generic bus crate cannot own: a `static`
//! typed to *this* application's event catalog. `tinybus::OnceBus<E>` is
//! generic over `E`, so there is no single type for it to declare a static of —
//! the host declares it, and gets the whole surface off it.
//!
//! ```ignore
//! use crate::core::bus::BUS;
//! use crate::core::events::DomainEvent;
//!
//! BUS.publish(DomainEvent::SystemStartup { component: "example".into() });
//! let _sub = BUS.subscribe(Arc::new(MyHandler));
//! let resp: Resp = BUS.native().request("agent.run_turn", req).await?;
//! ```
//!
//! # The two surfaces, and which to use
//!
//! | Need | Use |
//! | --- | --- |
//! | tell anyone who cares that something happened | [`BUS`]`.publish` |
//! | call another module with a payload that is plain data | [`BUS`]`.publish` + a reply event, or a native request |
//! | call another module passing `Arc`s, trait objects or channels | [`BUS`]`.native()` |
//! | call an out-of-process integration | a `tinybus::Proxy` off [`BUS`]`.get()?.connection()` |
//!
//! The native registry is **not** a legacy path. Events are serialized now, and
//! `AgentTurnRequest` carries `Arc<Vec<Box<dyn Tool>>>` and a live progress
//! channel; there is no encoding of those that survives a process boundary, so
//! that surface stays in-process permanently and by design.
//!
//! # Before initialisation
//!
//! Every accessor is safe before [`init`] runs: publishing is a no-op that logs
//! at `trace`, subscribing returns `None`. Unit tests that never stand a bus up
//! keep working, and an early-startup publish does not panic.

use tinybus::events::EventBusConfig;
// Consumers import the shared event contracts from TinyBus directly.
use tinybus::global::OnceBus;
use tinybus::version::{InterfaceVersion, PeerManifest, Version};

use crate::core::events::DomainEvent;

/// The object path OpenHuman's events are published beneath. Each event's
/// `domain()` is appended, so a peer can subscribe to one domain's subtree.
pub const EVENTS_ROOT: &str = "/ai/tinyhumans/openhuman/events";

/// The interface the event catalog is published under.
///
/// The interface identity stays stable across catalog versions; peers use
/// [`EVENTS_VERSION`] to detect incompatible catalogs.
pub const EVENTS_INTERFACE: &str = "ai.tinyhumans.openhuman.Events";

/// The version of the event catalog this build speaks.
///
/// Bump the minor for an added variant or field; bump the major for a removed
/// variant or another change that an older subscriber cannot parse. Peers
/// exchange this through [`manifest`], so a version skew between the kernel
/// and an out-of-process integration is reported at startup with both numbers
/// rather than as a decode failure later.
///
/// `1.1.0` added the MCP reconnect-supervisor variants (#5931).
/// `1.2.0` added `ActiveWorkspaceChanged` (#5966).
/// `1.3.0` retired `McpSetupSecretRequested` with the MCP setup agent; a
/// subscriber that still matches on it simply never sees one.
/// `1.4.0` is the assistant-UI-elements pass: additive fields on
/// `ApprovalRequested`/`ApprovalDecided` (`tool_call_id`, `expires_at`,
/// `thread_id`, `client_id`, `resolution`), `PlanReviewRequested`/
/// `PlanReviewDecided` (same additions), the `Artifact*` family
/// (`tool_call_id`, `request_id`), the `RunQueue*` family (`item_id`,
/// `text_preview`), `ThreadGoalUpdated` (`goal`), `ExternalTransferPending`
/// (`request_id`), the new `ThreadTodosChanged` and `ThreadRunModeChanged`
/// variants. All additions are optional/defaulted, so an older subscriber
/// keeps parsing what a newer publisher emits.
/// `1.6.0` retired `SubconsciousTriggerProcessed`, which nothing published
/// after the background-reasoning engine was removed.
/// `1.7.0` added `MemoryDriverChanged` (memory-engine switch, additive).
/// `1.8.0` is memory v2: it retires the v1 memory, memory-diff, memory-tree,
/// tree-summarizer and learning variants (`MemoryDriverBindFailed`,
/// `MemoryDriverChanged`, `MemoryGuardDenied`, `MemorySyncRequested`,
/// `MemorySyncStageChanged`, `MemoryIngestionStarted`/`Completed`,
/// `MemoryDiff*`, `DocumentCanonicalized`, `TreeSummarizer*`,
/// `MemoryTreeBuildProgress`, `CacheRebuilt`) — nothing publishes them any
/// more — and adds `ConversationTurnCommitted` (memory's conversation
/// ingestion) and `CronSystemJobDue` (host-owned cron jobs).
/// `1.9.0` adds an optional `agent_id` to `ApprovalRequested`,
/// `ApprovalDecided` and `FlowApprovalRequested` (additive).
/// `1.10.0` removes the first-run harness initialization progress events.
/// `1.11.0` removes `FlowScheduleTick`: the cron scheduler fires a flow's
/// schedule trigger through the schedule runtime's `TriggerSink` instead.
/// `1.12.0` adds optional `thread_id` and `client_id` to `MemoryStored` and
/// `MemoryRecalled` (additive): the chat routing was an ambient task-local.
/// `1.13.0` adds an optional `tenant` to `CredentialChanged`, `CronSystemJobDue`,
/// `ComposioConnectionCreated` and `ComposioConfigChanged` (additive): a
/// subscriber rebuilds its dispatch context from it instead of reading an
/// ambient one.
/// The canonical 2.0 catalog retired the desktop push-to-talk transcript
/// event; canonical 2.1 added inbound-event queue variants.
/// `3.0.0` removes `CronDeliveryRequested` (announce delivery now sends
/// synchronously through the cron channel bridge) and includes optional
/// `CronJobCompleted` tenant ownership.
/// `3.1.0` adds `InboundTurnCompleted` and `InboundTurnFailed` (additive): a
/// pinned inbound source's batch ran on its agent.
/// `3.2.0` adds connection token and nonce metadata to device tunnel frames.
pub const EVENTS_VERSION: Version = Version::new(3, 2, 0);

/// The bus. Initialised once by [`init`]; safe to touch before that.
pub static BUS: OnceBus<DomainEvent> = OnceBus::new();

/// Where the catalog lives on the bus.
pub fn config() -> EventBusConfig {
    EventBusConfig::new(EVENTS_ROOT, EVENTS_INTERFACE)
        .expect("the events root and interface constants are valid")
}

/// What this process tells the rest of the bus about itself.
///
/// Declared as both provided and consumed: OpenHuman publishes the catalog and
/// subscribes to it, so an integration that does either needs to check
/// compatibility in both directions.
pub fn manifest() -> PeerManifest {
    let interface = EVENTS_INTERFACE
        .try_into()
        .expect("the events interface constant is valid");
    PeerManifest::new("openhuman")
        .version(
            Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or_else(|_| Version::new(0, 0, 0)),
        )
        .provides(InterfaceVersion::provided(interface, EVENTS_VERSION))
        .consumes(InterfaceVersion::consumed(
            EVENTS_INTERFACE
                .try_into()
                .expect("the events interface constant is valid"),
            EVENTS_VERSION,
        ))
}

/// Initialise the bus with an in-process broker.
///
/// The default for a desktop build with no external integrations running. It is
/// a real broker over a real transport, so serialisation, match rules and
/// routing are all exercised exactly as they will be once an integration moves
/// out of process — which makes that move a deployment change rather than a
/// code path that has never run.
///
/// Idempotent: a second call returns the bus the first one created.
/// # Runtime affinity
///
/// The in-process broker and this process's connection to it are tokio tasks,
/// so they belong to the runtime that calls `init` **first**. That is fine in
/// production, where the bus is initialised on the runtime that then runs for
/// the life of the process. It is a trap under `cargo test`, where every
/// `#[tokio::test]` builds its own runtime: the first test to call `init` wins
/// the `OnceLock`, and when its runtime is torn down the bus it left behind is
/// attached to a dead reactor. Tests that need to observe events should build
/// their own with [`crate::core::bus_testing::isolated_bus`] rather than share
/// the singleton.
pub async fn init() -> tinybus::Result<()> {
    BUS.init_in_process(config()).await?;
    // Announcing is advisory: the manifest lets other peers check compatibility,
    // and nothing in this process depends on it having landed. A failure here
    // must not turn a working bus into a startup error.
    if let Err(e) = BUS.announce(&manifest()).await {
        tracing::warn!(error = %e, "[bus] could not announce the peer manifest");
    }
    tracing::info!(
        events_version = %EVENTS_VERSION,
        "[bus] initialised with an in-process broker"
    );
    Ok(())
}

/// Logs every event at `debug`. Ported from the bus that was replaced.
///
/// Registered once at startup by the channel runtime. It subscribes to every
/// domain deliberately — its whole purpose is answering "was this event ever
/// published", which a filtered subscriber cannot.
///
/// It logs the domain and variant name only, never the payload: events carry
/// user content (a channel message, a dictated voice transcript, a committed
/// turn, an agent error quoting either), which must not reach the log at any
/// level.
pub struct TracingSubscriber;

#[async_trait::async_trait]
impl tinybus::EventHandler<DomainEvent> for TracingSubscriber {
    fn name(&self) -> &str {
        "core::bus::tracing"
    }

    async fn handle(&self, event: &DomainEvent) {
        tracing::debug!(
            domain = event.domain(),
            event = event.variant_name(),
            "[bus] event published"
        );
    }
}

#[cfg(test)]
#[path = "bus_tests.rs"]
mod tests;
