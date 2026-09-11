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
use tinybus::global::OnceBus;
use tinybus::version::{InterfaceVersion, PeerManifest, Version};

use crate::core::events::DomainEvent;

/// The object path OpenHuman's events are published beneath. Each event's
/// `domain()` is appended, so a peer can subscribe to one domain's subtree.
pub const EVENTS_ROOT: &str = "/ai/tinyhumans/openhuman/events";

/// The interface the event catalog is published under.
///
/// A breaking change to the catalog ships as a **new interface name**
/// (`Events2`), never as a redefinition — see `tinybus`'s protocol rules.
pub const EVENTS_INTERFACE: &str = "ai.tinyhumans.openhuman.Events";

/// The version of the event catalog this build speaks.
///
/// Bump the minor for an added variant or field; bump the major only for a
/// change that an older subscriber cannot parse, and rename the interface when
/// you do. Peers exchange this through [`manifest`], so a version skew between
/// the kernel and an out-of-process integration is reported at startup with
/// both numbers rather than as a decode failure later.
///
/// `1.1.0` added the MCP reconnect-supervisor variants (#5931).
/// `1.2.0` added `ActiveWorkspaceChanged` (#5966).
pub const EVENTS_VERSION: Version = Version::new(1, 2, 0);

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

/// Initialise against a broker already listening on a socket.
///
/// Used when integrations run out of process: the kernel joins their bus rather
/// than standing up its own. `address` is the broker's socket path.
#[cfg(unix)]
pub async fn init_over_socket(address: impl AsRef<std::path::Path>) -> tinybus::Result<()> {
    let transport = tinybus::transport::unix::UnixTransport::connect(address.as_ref()).await?;
    BUS.init_over(Box::new(transport), config()).await?;
    if let Err(e) = BUS.announce(&manifest()).await {
        tracing::warn!(error = %e, "[bus] could not announce the peer manifest");
    }
    tracing::info!(
        address = %address.as_ref().display(),
        events_version = %EVENTS_VERSION,
        "[bus] initialised against a shared broker"
    );
    Ok(())
}

/// Logs every event at `debug`. Ported from the bus that was replaced.
///
/// Registered once at startup by the channel runtime. It subscribes to every
/// domain deliberately — its whole purpose is answering "was this event ever
/// published", which a filtered subscriber cannot.
pub struct TracingSubscriber;

#[async_trait::async_trait]
impl tinybus::EventHandler<DomainEvent> for TracingSubscriber {
    fn name(&self) -> &str {
        "core::bus::tracing"
    }

    async fn handle(&self, event: &DomainEvent) {
        tracing::debug!(domain = event.domain(), "[bus] {event:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_address_constants_are_valid() {
        // These are `expect`ed at runtime, so a typo must fail here rather than
        // at startup in a user's process.
        let _ = config();
        let _ = manifest();
    }

    #[test]
    fn the_manifest_declares_the_catalog_in_both_directions() {
        let manifest = manifest();
        let interface = EVENTS_INTERFACE.try_into().unwrap();
        assert!(
            manifest.provided(&interface).is_some(),
            "openhuman publishes"
        );
        assert!(
            manifest.consumed(&interface).is_some(),
            "openhuman subscribes"
        );
    }

    #[tokio::test]
    async fn isolated_bus_delivers() {
        use std::sync::Arc;
        use tinybus::EventHandler;
        use tokio::sync::Mutex;

        struct Capture(Arc<Mutex<Vec<DomainEvent>>>);

        #[async_trait::async_trait]
        impl EventHandler<DomainEvent> for Capture {
            fn name(&self) -> &str {
                "test::capture"
            }
            async fn handle(&self, event: &DomainEvent) {
                self.0.lock().await.push(event.clone());
            }
        }

        let bus = crate::core::bus_testing::isolated_bus().await;

        let seen = Arc::new(Mutex::new(Vec::new()));
        let _handle = bus.subscribe(Arc::new(Capture(seen.clone())));
        bus.publish(DomainEvent::SystemStartup {
            component: "test".into(),
        });

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while seen.lock().await.is_empty() {
            assert!(tokio::time::Instant::now() < deadline, "no delivery");
            tokio::task::yield_now().await;
        }
    }
}
