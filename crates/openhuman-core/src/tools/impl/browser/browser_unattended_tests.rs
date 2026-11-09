use super::super::pending::Pending;
use super::super::session_pool::{browser_session_fingerprint, thread_sessions, ThreadSession};
use super::super::{approve_browser_action, task_actions::approve_task_action, BrowserTool};
use super::*;
use crate::core::runtime::{CoreContext, DomainSet};
use crate::modules::browser::BrowserClient;
use crate::security::SecurityPolicy;
use serde_json::json;
use std::sync::Arc;
use std::time::Instant;
use tinycomputer_bus::agent::TaskId;
use tinycomputer_bus::browser::{Action, SessionId, Target};
use tinytools::Tool;

/// Tests that reach the allow-path log take this, so a log-capturing test's
/// scoped subscriber never races a sibling registering the same callsite.
static ALLOW_PATH: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const ALLOWED_LINE: &str = "[browser] unattended action allowed";

fn listing(actions: &[&str]) -> BrowserConfig {
    BrowserConfig {
        unattended_actions: actions.iter().map(|action| (*action).to_owned()).collect(),
        ..BrowserConfig::default()
    }
}

fn automation(source: TrustedAutomationSource) -> AgentTurnOrigin {
    AgentTurnOrigin::TrustedAutomation {
        job_id: "job-1".into(),
        source,
    }
}

fn cron() -> AgentTurnOrigin {
    automation(TrustedAutomationSource::Cron)
}

fn approval_workflow() -> AgentTurnOrigin {
    automation(TrustedAutomationSource::Workflow {
        require_approval: true,
    })
}

fn web_chat() -> AgentTurnOrigin {
    AgentTurnOrigin::WebChat {
        thread_id: "thread-1".into(),
        client_id: "client-1".into(),
        request_id: None,
    }
}

fn external_channel() -> AgentTurnOrigin {
    AgentTurnOrigin::ExternalChannel {
        channel: "telegram".into(),
        sender: Some("someone".into()),
        sender_name: None,
        reply_target: "chat-1".into(),
        message_id: "m-1".into(),
        history_key: None,
        channel_session: None,
    }
}

#[test]
fn trusted_unattended_sources_may_take_a_listed_action() {
    let browser = listing(&["click"]);
    for origin in [
        cron(),
        automation(TrustedAutomationSource::Background),
        automation(TrustedAutomationSource::Workflow {
            require_approval: false,
        }),
    ] {
        assert!(allowed_for(Some(&origin), &browser, "click"), "{origin:?}");
    }
}

#[test]
fn cron_may_not_take_an_action_that_is_not_listed() {
    assert!(!allowed_for(Some(&cron()), &listing(&["click"]), "fill"));
}

#[test]
fn an_empty_list_allows_nothing_for_any_origin() {
    let browser = BrowserConfig::default();
    for origin in [cron(), web_chat(), external_channel()] {
        assert!(!allowed_for(Some(&origin), &browser, "click"), "{origin:?}");
    }
}

#[test]
fn interactive_remote_and_unlabelled_turns_never_skip_the_gate() {
    let browser = listing(&["click", "task_step"]);
    for origin in [
        web_chat(),
        external_channel(),
        approval_workflow(),
        AgentTurnOrigin::Cli,
        AgentTurnOrigin::DirectChat,
        AgentTurnOrigin::Unknown,
    ] {
        assert!(!allowed_for(Some(&origin), &browser, "click"), "{origin:?}");
    }
    assert!(!allowed_for(None, &browser, "click"));
}

#[tokio::test]
async fn allow_decides_for_the_origin_it_is_given() {
    let _serial = ALLOW_PATH.lock().await;
    let browser = listing(&["press"]);
    assert!(allow(Some(&cron()), &browser, "press", "abc"));
    assert!(!allow(Some(&web_chat()), &browser, "press", "abc"));
    assert!(!allow(None, &browser, "press", "abc"));
}

fn pending() -> Pending {
    Pending {
        task: TaskId::new("t-1"),
        action: "Open the next page".into(),
        target: "Next".into(),
        token: "token".into(),
    }
}

#[tokio::test]
async fn a_listed_task_step_is_approved_for_cron_without_a_gate() {
    let _serial = ALLOW_PATH.lock().await;
    let approved = approve_task_action(&pending(), &listing(&["task_step"]), Some(&cron())).await;
    assert!(approved.unwrap());
}

#[tokio::test]
async fn an_unlisted_or_untrusted_task_step_still_needs_the_gate() {
    // With no interactive gate this errors; with one, the forced gate denies a
    // non-WebChat turn. Either way the step is not approved.
    let unlisted = approve_task_action(&pending(), &listing(&["click"]), Some(&cron())).await;
    assert!(!matches!(unlisted, Ok(true)));
    for origin in [external_channel(), approval_workflow()] {
        let outcome =
            approve_task_action(&pending(), &listing(&["task_step"]), Some(&origin)).await;
        assert!(!matches!(outcome, Ok(true)), "{origin:?}");
    }
}

fn offline_config(actions: &[&str]) -> crate::config::Config {
    let mut config = crate::config::Config::default();
    // No module and no download: a call that reached the module fails fast.
    config.modules.enabled = false;
    config.modules.allow_download = false;
    config.browser = listing(actions);
    config
}

fn offline_client(actions: &[&str]) -> BrowserClient {
    BrowserClient::new(Arc::new(offline_config(actions)))
}

fn click() -> Action {
    Action::Click {
        target: Target::reference("e1"),
        new_tab: false,
    }
}

#[tokio::test]
async fn a_listed_direct_action_runs_for_cron_without_reading_the_page() {
    let _serial = ALLOW_PATH.lock().await;
    let client = offline_client(&["click"]);
    let session = SessionId::new("s-1");
    let outcome = approve_browser_action(&client, &session, &click(), false, Some(&cron())).await;
    assert!(outcome.is_ok(), "{outcome:?}");
}

#[tokio::test]
async fn an_unlisted_or_untrusted_direct_action_is_not_waved_through() {
    let session = SessionId::new("s-1");
    let unlisted = offline_client(&["press"]);
    let outcome = approve_browser_action(&unlisted, &session, &click(), false, Some(&cron())).await;
    assert!(outcome.is_err());
    let listed = offline_client(&["click"]);
    for origin in [Some(external_channel()), Some(approval_workflow()), None] {
        let outcome =
            approve_browser_action(&listed, &session, &click(), false, origin.as_ref()).await;
        assert!(outcome.is_err(), "{origin:?}");
    }
}

#[derive(Clone, Default)]
struct Logs(Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Logs {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Logs {
    /// Capture this thread's logs while the returned guard lives.
    fn capture() -> (Self, tracing::subscriber::DefaultGuard) {
        let logs = Logs::default();
        let writer = logs.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .finish();
        let guard = tracing::subscriber::set_default(subscriber);
        tracing::callsite::rebuild_interest_cache();
        (logs, guard)
    }

    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

#[tokio::test]
async fn an_allowed_action_is_logged_by_kind_and_digest_without_its_input() {
    let _serial = ALLOW_PATH.lock().await;
    let (logs, _guard) = Logs::capture();
    let client = offline_client(&["fill"]);
    let fill = Action::Fill {
        target: Target::selector("#secret-field"),
        value: "hunter2-password".into(),
    };
    let session = SessionId::new("s-1");
    approve_browser_action(&client, &session, &fill, false, Some(&cron()))
        .await
        .unwrap();
    let text = logs.text();
    let line = text
        .lines()
        .find(|line| line.contains(ALLOWED_LINE))
        .unwrap_or_else(|| panic!("no unattended log line in {text:?}"));
    assert!(
        line.contains("action=\"fill\"") || line.contains("action=fill"),
        "{line}"
    );
    assert!(line.contains("action_digest="), "{line}");
    assert!(line.contains("TrustedAutomation(Cron)"), "{line}");
    assert!(
        !line.contains("hunter2-password") && !line.contains("#secret-field"),
        "{line}"
    );
    assert!(!line.contains("job-1"), "{line}");
}

/// Run one `click` through `BrowserTool::execute` — the production entry
/// point — inside a CoreContext scoped with `origin`, as a cron, workflow or
/// chat entry point scopes its turn. The thread already holds a browser
/// session, so nothing before the approval decision needs the module.
async fn dispatch_click(origin: AgentTurnOrigin, actions: &[&str]) -> (String, String) {
    let (logs, _guard) = Logs::capture();
    let client = Arc::new(offline_client(actions));
    let key = format!("unattended-test:{}", uuid::Uuid::new_v4());
    thread_sessions().lock().await.insert(
        key.clone(),
        ThreadSession {
            id: SessionId::new("held-browser"),
            config_fingerprint: browser_session_fingerprint(&client),
            client: client.clone(),
            last_used: Instant::now(),
            bound_origin: None,
        },
    );
    let tool = BrowserTool::new(Arc::new(SecurityPolicy::default()), client, 3);
    *tool.thread_key.lock().unwrap() = Some(key.clone());
    let context = CoreContext::for_test(DomainSet::full(), None);
    let result = CoreContext::scope_with_turn_origin(
        context,
        Some(origin),
        tool.execute(json!({"action": "click", "selector": "@e1"})),
    )
    .await
    .unwrap();
    thread_sessions().lock().await.remove(&key);
    (result.output(), logs.text())
}

#[tokio::test]
async fn the_tool_lets_a_listed_cron_click_past_approval_to_the_module() {
    let _serial = ALLOW_PATH.lock().await;
    let (output, logs) = dispatch_click(cron(), &["click"]).await;
    // Approval was skipped; the click reached `perform`, which fails only
    // because this test has no module.
    assert!(logs.contains(ALLOWED_LINE), "{logs}");
    assert!(!output.contains("policy-denied"), "{output}");
}

#[tokio::test]
async fn the_tool_keeps_every_other_turn_and_action_behind_the_gate() {
    let _serial = ALLOW_PATH.lock().await;
    for (origin, actions) in [
        (cron(), &["press"][..]),
        (cron(), &[][..]),
        (web_chat(), &["click"][..]),
        (external_channel(), &["click"][..]),
        (approval_workflow(), &["click"][..]),
        (AgentTurnOrigin::Unknown, &["click"][..]),
    ] {
        let (output, logs) = dispatch_click(origin.clone(), actions).await;
        assert!(!logs.contains(ALLOWED_LINE), "{origin:?}: {logs}");
        assert!(!output.is_empty(), "{origin:?}");
    }
}

#[tokio::test]
async fn allow_logs_the_canonical_kind_not_the_callers_string() {
    let _serial = ALLOW_PATH.lock().await;
    let (logs, _guard) = Logs::capture();
    assert!(allow(
        Some(&cron()),
        &listing(&["click"]),
        " CLICK ",
        "not-hex-digest"
    ));
    let text = logs.text();
    let line = text
        .lines()
        .find(|line| line.contains(ALLOWED_LINE))
        .unwrap();
    assert!(
        line.contains("action=\"click\"") || line.contains("action=click"),
        "{line}"
    );
    assert!(
        !line.contains(" CLICK ") && !line.contains("not-hex"),
        "{line}"
    );
}
