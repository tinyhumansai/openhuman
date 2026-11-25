//! TinyComputer end to end in a `security-module` build (TS-E2E-010): the
//! staged TinySecurity and TinyComputer releases, called through
//! `openhuman.modules_computer_status`. Run each test by exact name in its own
//! process after `source scripts/ci/ts-e2e-stage-modules.sh tinysecurity,tinycomputer`.

#[allow(dead_code)]
#[path = "support/ts_e2e_host.rs"]
mod ts_e2e_host;
#[allow(dead_code)]
#[path = "support/ts_e2e_modules.rs"]
mod ts_e2e_modules;

use ts_e2e_host::{run_on_agent_stack, Host};

/// One TinyComputer member call end to end: load the module and `Describe`
/// it through `desktop::proxy`, which pushes the browser egress config first.
async fn computer_status(host: &Host) -> serde_json::Value {
    host.rpc(
        "openhuman.modules_computer_status",
        serde_json::json!({"load": true}),
    )
    .await
}

/// The status reports a failed load or `Describe` in its own `error` field
/// inside a successful envelope, so only capabilities prove TinyComputer answered.
fn assert_computer_answers(envelope: &serde_json::Value, when: &str) {
    let status = Host::result(envelope);
    assert!(
        status["error"].is_null() && status["capabilities"].is_object(),
        "TinyComputer must answer {when}: {envelope}"
    );
}

/// CB-08: in a `security-module` build, a TinyComputer call succeeds. The
/// predicted gap: the module is loaded callback-bound without
/// `browser.egress`, and tinybus refuses `desktop::proxy`'s reinit with it
/// ("scoped callback configuration is immutable").
#[test]
#[ignore = "needs the staged TinySecurity and TinyComputer releases (scripts/ci/ts-e2e-stage-modules.sh)"]
fn cb_08_tinycomputer_answers_in_a_security_module_build() {
    run_on_agent_stack("cb_08", || async {
        ts_e2e_modules::require_staged("tinycomputer");
        let scratch = tempfile::tempdir().expect("scratch");
        let host = Host::start(scratch.path(), |_| {}).await;
        assert_computer_answers(&computer_status(&host).await, "a first call");
        assert_computer_answers(&computer_status(&host).await, "a second call");
        host.stop().await;
    });
}

/// CB-11: TinyComputer keeps answering after its browser egress policy is
/// re-registered (every 25 s), which changes the pushed config again.
#[test]
#[ignore = "needs the staged TinySecurity and TinyComputer releases (scripts/ci/ts-e2e-stage-modules.sh)"]
fn cb_11_tinycomputer_answers_after_the_egress_policy_refreshes() {
    run_on_agent_stack("cb_11", || async {
        ts_e2e_modules::require_staged("tinycomputer");
        let scratch = tempfile::tempdir().expect("scratch");
        let host = Host::start(scratch.path(), |_| {}).await;
        assert_computer_answers(&computer_status(&host).await, "before the refresh");
        tokio::time::sleep(std::time::Duration::from_secs(31)).await;
        assert_computer_answers(&computer_status(&host).await, "after the refresh");
        host.stop().await;
    });
}
