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
use std::sync::{Arc, MutexGuard};
use tinybus::module::ModuleClient;
#[cfg(feature = "e2e-test-support")]
use tinychannels_bus::methods;
use tokio::sync::OnceCell;

const MODULE_ID: &str = "tinychannels";
static CLIENT: OnceCell<ModuleClient> = OnceCell::const_new();

/// Return a client for the compiled, linked TinyChannels module fixture.
pub(crate) async fn client() -> ModuleClient {
    CLIENT
        .get_or_init(|| async {
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
        })
        .await
        .clone()
}

#[tokio::test]
async fn concurrent_linked_module_client_requests_share_the_ready_fixture() {
    let (first, second, third) = tokio::join!(client(), client(), client());

    assert!(first.fixture_proxy().is_some());
    assert!(second.fixture_proxy().is_some());
    assert!(third.fixture_proxy().is_some());
}

#[tokio::test]
async fn linked_module_accepts_confidential_web_projection_begin() {
    let module = client().await;
    let handle: tinychannels_bus::module_web_output::WebTurnHandle = module
        .call_confidential(
            "tinychannels",
            tinychannels_bus::methods::BEGIN_WEB_TURN,
            (tinychannels_bus::module_web_output::BeginWebTurn {
                scope_key: "fixture-web-chat:begin".into(),
                context: tinychannels_bus::web::WebEventContext {
                    client_id: "fixture-client".into(),
                    thread_id: "fixture-thread".into(),
                    request_id: "fixture-request".into(),
                    ..Default::default()
                },
            },),
        )
        .await
        .unwrap_or_else(|error| panic!("confidential web projection begin failed: {error:?}"));

    module
        .call_confidential::<()>(
            "tinychannels",
            tinychannels_bus::methods::END_WEB_TURN,
            (tinychannels_bus::module_web_output::EndWebTurn {
                handle: handle.handle,
            },),
        )
        .await
        .expect("the linked module should end its web projection turn");
}

/// Temporarily install in-memory storage for module callbacks in isolated tests.
pub(crate) struct MemoryStorageGuard {
    _slot_lock: MutexGuard<'static, ()>,
    previous: Option<Arc<dyn crate::storage::StorageBackend>>,
}

/// Give the linked module a writable backend for storage-backed fixture calls.
pub(crate) fn install_memory_storage() -> MemoryStorageGuard {
    let slot_lock = crate::storage::test_support::lock_storage_slot();
    MemoryStorageGuard {
        _slot_lock: slot_lock,
        // The module's media store and legacy-import claims ask for a
        // conditioned scope, which the plain memory engine cannot open; one
        // test process is the only writer, so the unfenced wrapper is exact.
        previous: crate::storage::install(std::sync::Arc::new(
            crate::storage::test_support::UnfencedMemoryBackend::new(),
        )),
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
                preparation_mode: Default::default(),
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
        "v22|namespace=p9:openhuman|channel=p7:discord|account=a|kind=p7:unknown|scope=a|conversation=p6:room#b|parent=a|topic=a|thread=p8:thread/c|participant=a"
    );
    assert_eq!(response.message.timestamp, 42);
    assert_eq!(response.message.sender_name.as_deref(), Some("Display"));
    assert_eq!(response.envelope.text, "fixture content");
}
