use super::*;

use super::binding::AgentRunRegistry;
use crate::config::Config;
use crate::core::runtime::{ComponentSet, CoreContext, DispatchCtx, DomainSet};
use std::sync::Arc;
use tempfile::TempDir;

pub(super) fn binding(ctx: DispatchCtx, temp: &TempDir) -> CapsHostBinding {
    let config = Config {
        workspace_dir: temp.path().join("workspace"),
        action_dir: temp.path().join("workspace"),
        config_path: temp.path().join("config.toml"),
        ..Config::default()
    };
    CapsHostBinding {
        ctx,
        config: Arc::new(config.clone()),
        flow_id: "flow-1".into(),
        run_id: "run-1".into(),
        security: Arc::new(crate::security::SecurityPolicy::from_config(
            &config.autonomy,
            &config.workspace_dir,
            &config.action_dir,
        )),
        approval: Default::default(),
        cancellation: tokio_util::sync::CancellationToken::new(),
        agent_runs: tokio::sync::Mutex::new(AgentRunRegistry::default()),
    }
}

#[tokio::test]
async fn callback_reenters_the_captured_context_instead_of_ambient_context() {
    let temp = TempDir::new().unwrap();
    let handed_off = DispatchCtx::new(CoreContext::for_test_with_components(ComponentSet {
        sandbox: false,
        ..ComponentSet::default()
    }));
    let ambient = CoreContext::for_test(DomainSet::full(), None);
    let binding = binding(handed_off, &temp);

    CoreContext::scope(ambient, async {
        assert!(DispatchCtx::ambient().components().sandbox);
        assert!(
            !binding
                .within(async { DispatchCtx::ambient().components().sandbox })
                .await
        );
    })
    .await;
}
