//! A compaction must not hold a turn for the summary budget when it asked for
//! no summary. Without one the module only does local work, so a reply that
//! never comes is a lost reply, and the turn passes the output through after
//! the ordinary TinyBus deadline instead of waiting out `COMPACT_WITH_TIMEOUT`.

use super::*;
use std::time::Duration;

use tinybus::{broker::Broker, transport::memory::MemoryBus, Connection, ObjectPath};
use tinyjuice_bus::wire::{CompactRequest, CompactResponse, InstallRequest};

/// A TinyJuice that installs and then never answers a compaction.
#[derive(Clone)]
struct SilentJuice;

#[tinybus::interface(name = "ai.tinyhumans.tinyjuice.Compression")]
impl SilentJuice {
    async fn install(&self, _request: InstallRequest) -> tinybus::Result<()> {
        Ok(())
    }

    async fn compact_with(&self, _request: CompactRequest) -> tinybus::Result<CompactResponse> {
        std::future::pending().await
    }
}

async fn silent_module() -> (tinybus::Proxy, Connection) {
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let service = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let caller = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    service
        .serve_at(
            ObjectPath::new(tinyjuice_bus::names::OBJECT_PATH).unwrap(),
            SilentJuice,
        )
        .await
        .unwrap();
    service
        .request_name(tinyjuice_bus::names::BUS_NAME)
        .await
        .unwrap();
    let proxy = caller
        .proxy(
            tinyjuice_bus::names::BUS_NAME,
            tinyjuice_bus::names::OBJECT_PATH,
            tinyjuice_bus::names::BUS_NAME,
        )
        .unwrap();
    (proxy, service)
}

fn call<'a>(
    config: &'a Arc<crate::config::Config>,
    context_token: Option<String>,
) -> ToolOutputCompaction<'a> {
    ToolOutputCompaction {
        content: "refused: outside the profile".into(),
        tool_name: "shell",
        enabled: true,
        profile: AgentTokenjuiceCompression::Auto,
        runtime_config: Some(config),
        arguments: None,
        focus: None,
        context_token,
        scope: Some("thread".into()),
    }
}

/// Runs `compact` against a module that never replies, and reports whether it
/// finished within `within` of (paused) time.
async fn finishes_within(
    context_token: Option<String>,
    within: Duration,
) -> Option<CompactedToolOutput> {
    let workspace = tempfile::tempdir().unwrap();
    let mut config = crate::config::Config::default();
    config.workspace_dir = workspace.path().to_owned();
    config.config_path = workspace.path().join("config.toml");
    let config = Arc::new(config);
    let (proxy, _service) = silent_module().await;
    let fixture = module_stub::RpcClientFixture {
        proxy,
        configs: Default::default(),
    };
    module_stub::with_rpc_client(fixture, async {
        tokio::time::timeout(within, compact_tool_output(call(&config, context_token)))
            .await
            .ok()
    })
    .await
}

#[tokio::test(start_paused = true)]
async fn a_compaction_without_a_summary_passes_through_when_the_module_never_answers() {
    let compacted = finishes_within(None, Duration::from_secs(60))
        .await
        .expect("a lost reply must not hold the turn for the summary budget");
    assert_eq!(compacted.text, "refused: outside the profile");
    assert!(compacted.summarized_from_bytes.is_none());
}

#[tokio::test(start_paused = true)]
async fn a_compaction_that_asks_for_a_summary_keeps_the_summary_budget() {
    assert!(
        finishes_within(Some("ticket".into()), Duration::from_secs(60))
            .await
            .is_none(),
        "a summary may legitimately take minutes; it keeps COMPACT_WITH_TIMEOUT"
    );
}
