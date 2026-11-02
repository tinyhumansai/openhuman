use super::*;
use crate::config::Config;
use crate::flows::http_credentials::{HttpCredential, HttpCredentialsStore};
use crate::security::SecurityPolicy;

fn http_cred_store() -> (tempfile::TempDir, HttpCredentialsStore) {
    let dir = tempfile::tempdir().expect("tempdir");
    // The focused flow tests exercise credential routing, not TinySecurity crypto.
    let store = HttpCredentialsStore::new(dir.path(), false);
    (dir, store)
}

// ── Phase 2: autonomy-tier gating of acting nodes ──────────────────────

fn policy(level: crate::security::AutonomyLevel) -> SecurityPolicy {
    SecurityPolicy {
        autonomy: level,
        ..SecurityPolicy::default()
    }
}

// ── Codex P1: Prompt-tier decisions must escalate past a workflow's own
// require_approval=false default, never silently auto-allow ────────────

use crate::core::runtime::turn_origin::{AgentTurnOrigin, TrustedAutomationSource};

fn workflow_origin(job_id: &str, require_approval: bool) -> AgentTurnOrigin {
    AgentTurnOrigin::TrustedAutomation {
        job_id: job_id.to_string(),
        source: TrustedAutomationSource::Workflow { require_approval },
    }
}

fn agent_test_config(tmp: &tempfile::TempDir) -> Config {
    let mut config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    config.default_model = Some("test/session-fixture".to_string());
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    let fixture_config = config.clone();
    crate::storage::block_on_anyhow(async move {
        crate::agent::session_host::test_support::install_managed_model_fixture(&fixture_config)
            .await;
        Ok(())
    })
    .expect("install managed model fixture");
    config
}

#[path = "ops_schema_and_structured_output_tests.rs"]
mod schema_and_structured_output_tests;
#[path = "ops_tier_gate_and_timeouts_tests.rs"]
mod tier_gate_and_timeouts_tests;
#[path = "ops_tool_results_and_credentials_tests.rs"]
mod tool_results_and_credentials_tests;
