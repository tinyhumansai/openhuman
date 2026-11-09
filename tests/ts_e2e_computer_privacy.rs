//! TinyComputer under `[privacy] mode = "local_only"` with the staged
//! TinySecurity and TinyComputer releases (TS-E2E-011). Run by exact name after
//! `source scripts/ci/ts-e2e-stage-modules.sh tinysecurity,tinycomputer`.

#[allow(dead_code)]
#[path = "support/ts_e2e_host.rs"]
mod ts_e2e_host;
#[allow(dead_code)]
#[path = "support/ts_e2e_modules.rs"]
mod ts_e2e_modules;

use ts_e2e_host::{run_on_agent_stack, Host};

/// LocalOnly keeps TinyComputer's browser egress policy registrable: a call no
/// longer fails with "TinySecurity browser egress policy unavailable", which
/// TinySecurity's refusal of an open policy under LocalOnly caused. (Until
/// TS-E2E-010 lands, the call then stops at the callback config refresh; this
/// row asserts only the egress policy.)
#[test]
#[ignore = "needs the staged TinySecurity and TinyComputer releases (scripts/ci/ts-e2e-stage-modules.sh)"]
fn local_only_keeps_the_tinycomputer_browser_policy_registrable() {
    run_on_agent_stack("computer_local_only", || async {
        ts_e2e_modules::require_staged("tinycomputer");
        let scratch = tempfile::tempdir().expect("scratch");
        let host = Host::start(scratch.path(), |config| {
            config.privacy.mode = openhuman_core::config::PrivacyMode::LocalOnly;
        })
        .await;
        let envelope = host
            .rpc(
                "openhuman.modules_computer_status",
                serde_json::json!({"load": true}),
            )
            .await;
        // A failed load or call is reported in the status's own `error`, so
        // require evidence the call got past policy registration: capabilities,
        // or the config refresh that follows it (TS-E2E-010, until it lands).
        let status = Host::result(&envelope);
        let error = status["error"].as_str().unwrap_or_default();
        let past_registration =
            status["capabilities"].is_object() || error.contains("configuration refresh failed");
        assert!(
            past_registration && !error.contains("browser egress policy unavailable"),
            "LocalOnly must not make the browser egress policy unavailable: {envelope}"
        );
        host.stop().await;
    });
}
