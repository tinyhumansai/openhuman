//! TinySecurity is admitted with one secret scope per process. While the
//! runtime that admitted it is live, a config that derives a different secret
//! scope (another config directory) is refused the module, so one live scope
//! is never handed another's master key, and the refusal leaves the live
//! runtime's own screening working.
//!
//! Needs the TinySecurity release on `OPENHUMAN_MODULE_PATH` (or in the module
//! release cache), so it is ignored by default and run by
//! `scripts/ci/security-native-fixture.sh`.

mod common;

use std::sync::Arc;

use common::{chat_requests, offline_config, provider, runtime, stub_backend};
use openhuman_embed::{AgentSpec, Provider, Runtime, Workspace};
use tinyagents_session::port::InMemorySessionStores;

/// A plain request the prompt screen lets through.
const BENIGN: &str = "What is the capital of France?";

#[test]
#[ignore = "needs the TinySecurity release; run by scripts/ci/security-native-fixture.sh"]
fn a_different_secret_scope_is_refused_while_the_admitting_runtime_is_live() {
    runtime().block_on(async {
        tokio::spawn(async {
            let backend = stub_backend().await;
            let model = provider("Paris").await;
            let runtime = Runtime::builder()
                .config(offline_config())
                .workspace(Workspace::Stateless)
                .session_store(Arc::new(InMemorySessionStores::new()))
                .backend_url(backend.uri())
                .build()
                .await
                .expect("runtime");
            let agent = runtime
                .agent(
                    AgentSpec::new("live").provider(
                        Provider::openai_compatible(format!("{}/v1", model.uri()), "fixture")
                            .model("fixture"),
                    ),
                )
                .expect("agent");
            let first = agent.turn(BENIGN).send().await.expect("screened turn");
            assert_eq!(first.reply, "Paris");

            let elsewhere = tempfile::tempdir().expect("other config directory");
            let mut other = offline_config();
            other.config_path = elsewhere.path().join("config.toml");
            other.workspace_dir = elsewhere.path().join("workspace");
            let refused = openhuman_core::modules::security::ensure_loaded(&other)
                .await
                .expect_err("another secret scope is refused the admitted module");
            assert!(
                refused
                    .to_string()
                    .contains("security secret scope changed after module admission"),
                "{refused}"
            );

            let second = agent
                .turn(BENIGN)
                .send()
                .await
                .expect("the live runtime still screens");
            assert_eq!(second.reply, "Paris");
            assert_eq!(chat_requests(&model).await.len(), 2);
            runtime.remove_agent("live").await.expect("remove agent");
            drop(agent);
        })
        .await
        .expect("test task");
    });
}
