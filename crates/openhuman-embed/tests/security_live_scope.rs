//! TinySecurity boots with a neutral secret authority, and each secret scope
//! (config directory) registers its own approval scope and secret
//! registration on it at runtime, with its own cipher, consumer and audit
//! subject. While the first runtime is live, a config that derives a
//! different secret scope gets a registration of its own instead of being
//! refused (and never the first one's master key), and the first runtime's
//! own screening keeps working.
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
fn a_different_secret_scope_gets_its_own_registration_while_the_first_runtime_is_live() {
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
            openhuman_core::modules::security::ensure_loaded(&other)
                .await
                .expect("another secret scope registers its own runtime registration");

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
