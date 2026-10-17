//! A linked TinyChannels module fixture for host-side unit tests.
//!
//! These tests exercise the real owning module over TinyBus without downloading
//! a release artifact or reaching a provider service. The module is attached
//! through the same seam local development uses: the factory is registered with
//! [`register_linked`] and `[modules] source = "linked"` makes
//! `ops::ensure_loaded` attach it to the process-wide module host, which lives
//! on a process-lifetime runtime, so it outlives the short Tokio runtimes of
//! individual tests.

use crate::config::schema::ModuleSourceMode;
use crate::config::Config;
use crate::modules::linked::register_linked;
use crate::modules::{ops, runtime};
use std::sync::Arc;
use tinybus::module::ModuleClient;
#[cfg(feature = "e2e-test-support")]
use tinychannels_bus::methods;

const MODULE_ID: &str = "tinychannels";

/// Return a client for the compiled, linked TinyChannels module fixture.
pub(crate) async fn client() -> ModuleClient {
    register_linked(MODULE_ID, tinychannels_module::linked_module);
    let mut config = Config::default();
    config.modules.allow_download = false;
    config.modules.source = ModuleSourceMode::Linked;
    ops::ensure_loaded(&config, MODULE_ID)
        .await
        .expect("the linked TinyChannels module should be admitted through the seam");
    let proxy = runtime::runtime()
        .await
        .expect("the module host should be running")
        .proxy(
            tinychannels_module::BUS_NAME,
            tinychannels_module::OBJECT_PATH,
        )
        .expect("the linked TinyChannels proxy should be valid");
    ops::fixture_client(proxy)
}

/// Temporarily install in-memory storage for module callbacks in isolated tests.
pub(crate) struct MemoryStorageGuard {
    previous: Option<Arc<dyn crate::storage::StorageBackend>>,
}

/// Give the linked module a writable backend for storage-backed fixture calls.
pub(crate) fn install_memory_storage() -> MemoryStorageGuard {
    MemoryStorageGuard {
        previous: crate::storage::install(Arc::new(crate::storage::MemoryStorage::new())),
    }
}

impl Drop for MemoryStorageGuard {
    fn drop(&mut self) {
        crate::storage::clear();
        if let Some(previous) = self.previous.take() {
            crate::storage::install(previous);
        }
    }
}

#[tokio::test]
#[cfg(feature = "e2e-test-support")]
async fn linked_module_prepares_runtime_limits_over_confidential_bus_call() {
    let _storage = install_memory_storage();
    use tinychannels_bus::module_control::{ChannelControlRequest, ChannelControlResult};

    let response: ChannelControlResult = client()
        .await
        .call_confidential(
            "tinychannels",
            methods::PREPARE_CHANNEL_CONTROL,
            (ChannelControlRequest::RuntimeLimits {
                timeout_secs: 1,
                listener_count: 100,
            },),
        )
        .await
        .expect("the attached linked module should answer a confidential call");

    assert_eq!(
        response,
        ChannelControlResult::RuntimeLimits {
            timeout_secs: 30,
            max_in_flight: 64,
        }
    );
}

#[tokio::test]
#[cfg(feature = "e2e-test-support")]
async fn linked_module_prepares_legacy_inbound_facts_over_confidential_bus_call() {
    let _storage = install_memory_storage();
    use tinychannels_bus::module_inbound::{
        ChannelInboundSource, PrepareChannelInboundRequest, PreparedChannelInbound,
    };
    use tinychannels_bus::ChannelMessage;

    let message = ChannelMessage {
        id: "message-1".into(),
        channel: "discord".into(),
        sender: "sender/a".into(),
        reply_target: "room#b".into(),
        content: "fixture content".into(),
        timestamp: 42,
        thread_ts: Some("thread/c".into()),
        sender_name: Some("Display".into()),
    };
    let response: PreparedChannelInbound = client()
        .await
        .call_confidential(
            "tinychannels",
            methods::PREPARE_CHANNEL_INBOUND,
            (PrepareChannelInboundRequest {
                source: ChannelInboundSource::Legacy {
                    message,
                    envelope: None,
                },
                namespace: "openhuman".into(),
                session_policy: Default::default(),
            },),
        )
        .await
        .expect("the attached linked module should prepare inbound facts");

    assert_eq!(
        response.thread_id,
        "channel:discord/sender%2Fa/room%23b#thread:thread%2Fc"
    );
    assert_eq!(response.client_id, "inbound:discord:sender/a");
    assert_eq!(
        response.history_key,
        "discord_sender/a_room#b_thread:thread/c"
    );
    assert_eq!(
        response.session_key,
        "openhuman:discord:default:unknown:room#b:thread/c"
    );
    assert_eq!(response.message.timestamp, 42);
    assert_eq!(response.message.sender_name.as_deref(), Some("Display"));
    assert_eq!(response.envelope.text, "fixture content");
}
