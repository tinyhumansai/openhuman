//! TinySecurity is admitted once per process and keeps the secret scope it was
//! admitted with. An embedded [`Runtime`] owns a fresh scratch config directory,
//! so a runtime built after an earlier one has been released derives a
//! different scope from its own config. It must still reach the admitted
//! module: its prompts are screened and pass, rather than being held for
//! security review because the module "belongs" to a runtime that is gone.
//!
//! Needs the TinySecurity release on `OPENHUMAN_MODULE_PATH` (or in the module
//! release cache), so it is ignored by default and run by
//! `scripts/ci/security-native-fixture.sh`.

mod common;

use common::{chat_requests, offline_config, provider, runtime, stub_backend};
use openhuman_embed::{AgentSpec, Provider, Runtime, Workspace};

/// A plain request the prompt screen lets through.
const BENIGN: &str = "What is the capital of France?";

/// Build a runtime with the prompt screen enforced, send one benign turn and
/// release the runtime. Returns the turn's reply, or the error it failed with.
async fn screened_turn(name: &str) -> Result<String, String> {
    let backend = stub_backend().await;
    let model = provider("Paris").await;
    let runtime = Runtime::builder()
        .config(offline_config())
        .workspace(Workspace::Ephemeral)
        .backend_url(backend.uri())
        .build()
        .await
        .map_err(|error| format!("{name} runtime: {error}"))?;
    assert!(
        runtime.capabilities().configuration.security_enforced,
        "{name} runtime enforces security"
    );
    let agent = runtime
        .agent(
            AgentSpec::new(name).provider(
                Provider::openai_compatible(format!("{}/v1", model.uri()), "fixture")
                    .model("fixture"),
            ),
        )
        .map_err(|error| format!("{name} agent: {error}"))?;
    let outcome = agent.turn(BENIGN).send().await;
    let requests = chat_requests(&model).await.len();
    runtime.remove_agent(name).await.expect("remove agent");
    drop(agent);
    drop(runtime);
    let outcome = outcome.map_err(|error| format!("{name} turn: {error:?}"))?;
    if requests != 1 {
        return Err(format!(
            "{name} turn never reached the model ({requests} requests): {}",
            outcome.reply
        ));
    }
    Ok(outcome.reply)
}

#[test]
#[ignore = "needs the TinySecurity release; run by scripts/ci/security-native-fixture.sh"]
fn a_runtime_built_after_another_is_released_still_screens_its_prompts() {
    runtime().block_on(async {
        tokio::spawn(async {
            for name in ["first", "second"] {
                let outcome = screened_turn(name).await;
                eprintln!("{name} runtime: {outcome:?}");
                assert_eq!(outcome.as_deref(), Ok("Paris"), "{name} runtime");
            }
        })
        .await
        .expect("test task");
    });
}
