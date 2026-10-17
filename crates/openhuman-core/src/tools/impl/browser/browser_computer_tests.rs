use super::*;
use tinycomputer_bus::browser::{LocateBy, Locator, Target};

fn session_entry(client: Arc<BrowserClient>, id: SessionId, last_used: Instant) -> ThreadSession {
    let config_fingerprint = browser_session_fingerprint(&client);
    ThreadSession {
        id,
        client,
        last_used,
        config_fingerprint,
        bound_origin: None,
    }
}

#[test]
fn browser_tools_are_deferred_and_task_is_bounded() {
    let client = Arc::new(BrowserClient::new(Arc::new(
        crate::config::Config::default(),
    )));
    let tool = BrowserTool::new(Arc::new(SecurityPolicy::default()), client, 0);
    assert_eq!(tool.exposure(), tinytools::ToolExposure::Deferred);
    assert_eq!(tool.max_steps, 1);
    let actions = tool.parameters_schema()["properties"]["action"]["enum"]
        .as_array()
        .unwrap()
        .clone();
    assert!(actions.contains(&json!("task")));
    assert!(actions.contains(&json!("confirm_pending")));
    assert!(actions.contains(&json!("task_continue")));
    assert!(actions.contains(&json!("task_cancel")));
    // The tool gates immediately before dispatch, including controller steps.
    // The outer middleware must not prompt a second time for the same action.
    assert!(!tool.external_effect_with_args(&json!({"action":"confirm_pending"})));
    assert!(!tool.external_effect_with_args(&json!({"action":"click","selector":"@e1"})));
    assert!(!tool.external_effect_with_args(&json!({"action":"find","find_action":"fill"})));
    assert!(!tool.external_effect_with_args(&json!({"action":"type"})));
    assert!(!tool.external_effect_with_args(&json!({"action":"task"})));
}

#[test]
fn every_direct_or_controller_action_that_can_submit_needs_confirmation() {
    assert!(needs_host_confirmation(&Action::Click {
        target: Target::reference("e1"),
        new_tab: false
    }));
    assert!(needs_host_confirmation(&Action::Fill {
        target: Target::reference("e1"),
        value: "x".into()
    }));
    assert!(needs_host_confirmation(&Action::Type {
        target: None,
        text: "\n".into(),
        delay_ms: None
    }));
    assert!(needs_host_confirmation(&Action::Press {
        key: "Enter".into(),
        target: None
    }));
    assert!(!needs_host_confirmation(&Action::Hover {
        target: Target::reference("e1")
    }));
}

#[test]
fn approval_describes_selector_and_locator_targets() {
    let click = |selector: &str| Action::Click {
        target: Target::selector(selector),
        new_tab: false,
    };
    let submit = click("#submit");
    let (reference, first) = approval_target(&submit);
    let (_, second) = approval_target(&click("#delete"));
    assert!(reference.is_none());
    assert!(first.contains("CSS selector") && first.contains("#submit"));
    assert_ne!(first, second);

    let (_, locator) = approval_target(&Action::Fill {
        target: Target::locator(Locator {
            by: LocateBy::Role,
            value: "textbox".into(),
            name: Some("Email".into()),
            ..Locator::default()
        }),
        value: "private@example.com".into(),
    });
    assert!(locator.contains("Role locator") && locator.contains("Email"));
    assert!(!locator.contains("private@example.com"));

    let press_action = Action::Press {
        key: "Enter".into(),
        target: Some(Target::reference("e7")),
    };
    let (reference, press) = approval_target(&press_action);
    assert_eq!(reference, Some("e7"));
    assert_eq!(press, " @e7");
    let (_, untargeted) = approval_target(&Action::Press {
        key: "Enter".into(),
        target: None,
    });
    assert!(untargeted.is_empty());
}

#[test]
fn direct_actions_decode_through_the_bus_tool_args() {
    assert!(
        matches!(direct_action(&json!({"action":"click","selector":"@e1"})).unwrap(), Action::Click { target: Target::Ref { value }, .. } if value == "e1")
    );
    assert!(direct_action(&json!({"action":"click"})).is_err());
    assert!(direct_action(&json!({"action":"scroll","direction":"diagonal"})).is_err());
    assert!(direct_action(&json!({"action":"arbitrary_script"})).is_err());
    assert!(direct_action(
        &json!({"action":"find","by":"text","value":"Save","find_action":"text"})
    )
    .is_ok());
}

#[test]
fn model_visible_browser_schema_is_the_bus_schema() {
    let tool = BrowserTool::new(
        std::sync::Arc::new(crate::security::SecurityPolicy::default()),
        std::sync::Arc::new(BrowserClient::new(std::sync::Arc::new(
            crate::config::Config::default(),
        ))),
        3,
    );
    let schema = tinytools::Tool::parameters_schema(&tool);
    assert_eq!(schema, tinycomputer_bus::browser::tool::parameters_schema());
    assert_eq!(schema["required"], json!(["action"]));
    assert_eq!(
        schema["properties"]["action"]["enum"]
            .as_array()
            .unwrap()
            .len(),
        22
    );
}

#[test]
fn confirmation_is_bound_to_one_pending_task_token() {
    let pending = Pending {
        task: TaskId::new("t-1"),
        action: "Submit the booking".into(),
        target: "Confirm button".into(),
        token: "token-one".into(),
    };
    assert!(pending.matches(&json!({"token":"token-one"})));
    assert!(!pending.matches(&json!({"token":"token-two"})));
    assert!(!pending.matches(&json!({})));
}

#[tokio::test]
async fn needs_approval_view_is_held_behind_a_token_and_cleared_by_other_states() {
    let client = Arc::new(BrowserClient::new(Arc::new(
        crate::config::Config::default(),
    )));
    let tool = BrowserTool::new(Arc::new(SecurityPolicy::default()), client, 3);
    let paused: TaskView = serde_json::from_value(json!({
        "id": "t-1",
        "status": {"state": "needs_approval", "action": "Send the message", "target": "Send"},
        "summary": "Ready to send.",
        "progress": 0.9,
        "next": ["ContinueTask"]
    }))
    .unwrap();
    let output = tool.report(paused).await.unwrap();
    let token = output["pending"]["token"].as_str().unwrap().to_owned();
    assert_eq!(output["pending"]["task_id"], "t-1");
    assert!(tool
        .pending
        .lock()
        .await
        .as_ref()
        .unwrap()
        .matches(&json!({"token": token})));

    let done: TaskView = serde_json::from_value(json!({
        "id": "t-1",
        "status": {"state": "done", "answer": "Sent.", "records": {}},
        "summary": "Sent.",
        "progress": 1.0,
        "next": []
    }))
    .unwrap();
    let output = tool.report(done).await.unwrap();
    assert!(output.get("pending").is_none());
    assert!(tool.pending.lock().await.is_none());
}

#[tokio::test]
async fn confirm_pending_without_a_held_action_is_refused() {
    let client = Arc::new(BrowserClient::new(Arc::new(
        crate::config::Config::default(),
    )));
    let tool = BrowserTool::new(Arc::new(SecurityPolicy::default()), client, 3);
    let error = tool
        .run(&json!({"action":"confirm_pending","token":"x"}), None)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("No pending"), "{error}");
}

#[tokio::test]
async fn task_inputs_must_be_text() {
    assert_eq!(
        task_inputs(&json!({"inputs":{"name":"Asha"}})).unwrap()["name"],
        "Asha"
    );
    assert!(task_inputs(&json!({"inputs":{"age":3}})).is_err());
    assert!(task_inputs(&json!({})).unwrap().is_empty());
}

#[tokio::test]
async fn task_without_allowed_websites_is_refused_before_the_module() {
    let client = Arc::new(BrowserClient::new(Arc::new(
        crate::config::Config::default(),
    )));
    if client.task_origins().is_empty() {
        let tool = BrowserTool::new(Arc::new(SecurityPolicy::default()), client, 3);
        let error = tool
            .run(&json!({"action":"task","goal":"Read the news"}), None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("allowed origin"), "{error}");
    }
}

#[tokio::test]
async fn a_second_turn_reuses_the_thread_session_and_close_removes_it() {
    let client = Arc::new(BrowserClient::new(Arc::new(
        crate::config::Config::default(),
    )));
    let key = format!("test:{}", uuid::Uuid::new_v4());
    let id = SessionId::new("existing-browser");
    thread_sessions().lock().await.insert(
        key.clone(),
        session_entry(client.clone(), id.clone(), Instant::now()),
    );
    let first = BrowserTool::new(Arc::new(SecurityPolicy::default()), client.clone(), 3);
    *first.thread_key.lock().unwrap() = Some(key.clone());
    assert_eq!(first.session().await.unwrap(), id);
    drop(first);
    let second = BrowserTool::new(Arc::new(SecurityPolicy::default()), client, 3);
    *second.thread_key.lock().unwrap() = Some(key.clone());
    assert_eq!(second.session().await.unwrap(), id);
    // The module is absent in this test; close still removes the thread entry
    // before the best-effort module call reports its availability error.
    let _ = second.close().await;
    assert!(!thread_sessions().lock().await.contains_key(&key));
}

#[tokio::test]
async fn evicted_session_is_not_reused_by_the_same_tool() {
    let client = Arc::new(BrowserClient::new(Arc::new(
        crate::config::Config::default(),
    )));
    let key = format!("test:{}", uuid::Uuid::new_v4());
    let old = SessionId::new("evicted-browser");
    let replacement = SessionId::new("replacement-browser");
    let tool = BrowserTool::new(Arc::new(SecurityPolicy::default()), client.clone(), 3);
    *tool.thread_key.lock().unwrap() = Some(key.clone());
    thread_sessions().lock().await.insert(
        key.clone(),
        session_entry(client.clone(), old.clone(), Instant::now()),
    );
    assert_eq!(tool.session().await.unwrap(), old);
    // Another turn evicts the entry and opens a replacement before this tool
    // next runs. The cached local handle must follow the pool's live entry.
    thread_sessions().lock().await.remove(&key);
    thread_sessions().lock().await.insert(
        key.clone(),
        session_entry(client.clone(), replacement.clone(), Instant::now()),
    );
    assert_eq!(tool.session().await.unwrap(), replacement);
    thread_sessions().lock().await.remove(&key);
}

#[tokio::test]
async fn stale_tool_close_preserves_a_newer_thread_session() {
    let client = Arc::new(BrowserClient::new(Arc::new(
        crate::config::Config::default(),
    )));
    let key = format!("test:{}", uuid::Uuid::new_v4());
    let tool = BrowserTool::new(Arc::new(SecurityPolicy::default()), client.clone(), 3);
    *tool.thread_key.lock().unwrap() = Some(key.clone());
    *tool.session.lock().await = Some(SessionId::new("old-browser"));
    thread_sessions().lock().await.insert(
        key.clone(),
        session_entry(client, SessionId::new("new-browser"), Instant::now()),
    );
    tool.close().await.unwrap();
    assert_eq!(
        thread_sessions()
            .lock()
            .await
            .get(&key)
            .unwrap()
            .id
            .as_str(),
        "new-browser"
    );
    thread_sessions().lock().await.remove(&key);
}

#[tokio::test]
async fn changed_browser_policy_retires_cached_session_for_same_thread() {
    let mut old_config = crate::config::Config::default();
    old_config.http_request.allowed_domains = vec!["example.com".into(), "other.test".into()];
    let old_client = Arc::new(BrowserClient::new(Arc::new(old_config)));
    let mut new_config = crate::config::Config::default();
    new_config.http_request.allowed_domains = vec!["example.com".into()];
    assert_ne!(
        browser_session_fingerprint(&old_client),
        browser_session_fingerprint(&BrowserClient::new(Arc::new(new_config.clone())))
    );
    new_config.browser.profile_mode = "persistent".into();
    new_config.browser.profile_path = Some("/tmp/browser-policy-test".into());
    new_config.browser.download_dir = Some("/tmp/browser-downloads-test".into());
    let new_client = Arc::new(BrowserClient::new(Arc::new(new_config)));
    assert_ne!(
        browser_session_fingerprint(&old_client),
        browser_session_fingerprint(&new_client)
    );

    let key = format!("test:{}", uuid::Uuid::new_v4());
    let old_id = SessionId::new("old-policy-browser");
    thread_sessions().lock().await.insert(
        key.clone(),
        session_entry(old_client, old_id.clone(), Instant::now()),
    );
    let tool = BrowserTool::new(Arc::new(SecurityPolicy::default()), new_client, 3);
    *tool.thread_key.lock().unwrap() = Some(key.clone());
    *tool.session.lock().await = Some(old_id.clone());
    let result = tool.session().await;
    assert_ne!(result.as_ref().ok(), Some(&old_id));
    assert_ne!(
        tool.session.lock().await.as_ref(),
        Some(&SessionId::new("old-policy-browser"))
    );
    // With no module running, close fails and the old entry stays quarantined
    // for a retry. If a module is available, the replacement has new settings.
    let sessions = thread_sessions().lock().await;
    match sessions.get(&key) {
        Some(entry) if entry.id == old_id => assert!(result.is_err()),
        Some(entry) => assert_eq!(
            entry.config_fingerprint,
            browser_session_fingerprint(&tool.client)
        ),
        None => assert!(result.is_err()),
    }
    drop(sessions);
    thread_sessions().lock().await.remove(&key);
}

#[test]
fn session_fingerprint_covers_browser_launch_settings() {
    let base = crate::config::Config::default();
    let original = browser_session_fingerprint(&BrowserClient::new(Arc::new(base.clone())));
    let mut changed = base;
    changed.browser.headless = !changed.browser.headless;
    assert_ne!(
        original,
        browser_session_fingerprint(&BrowserClient::new(Arc::new(changed)))
    );
}

#[test]
fn explicit_host_change_requires_a_new_session_but_other_actions_reuse_it() {
    let first = "https://.example.com";
    let second = "https://.other.example";
    assert!(requires_rebind(None, Some(first)));
    assert!(!requires_rebind(Some(first), Some(first)));
    assert!(!requires_rebind(Some(first), None));
    assert!(requires_rebind(Some(first), Some(second)));
}

#[test]
fn idle_and_oldest_sessions_are_bounded() {
    let client = Arc::new(BrowserClient::new(Arc::new(
        crate::config::Config::default(),
    )));
    let now = Instant::now();
    let mut sessions = HashMap::new();
    for index in 0..MAX_THREAD_SESSIONS {
        sessions.insert(
            format!("thread-{index}"),
            session_entry(
                client.clone(),
                SessionId::new(format!("browser-{index}")),
                now - Duration::from_secs(index as u64 + 1),
            ),
        );
    }
    let evicted = evict_thread_sessions(&mut sessions, now, true);
    assert_eq!(evicted.len(), 1);
    assert_eq!(evicted[0].id.as_str(), "browser-5");
    assert_eq!(sessions.len(), MAX_THREAD_SESSIONS - 1);
    sessions.insert(
        "expired".into(),
        session_entry(
            client,
            SessionId::new("expired-browser"),
            now - SESSION_IDLE_TTL,
        ),
    );
    let evicted = evict_thread_sessions(&mut sessions, now, false);
    assert_eq!(evicted.len(), 1);
    assert_eq!(evicted[0].id.as_str(), "expired-browser");
}

#[tokio::test]
async fn task_rejects_a_malformed_flow_before_the_module() {
    let client = Arc::new(BrowserClient::new(Arc::new(
        crate::config::Config::default(),
    )));
    let tool = BrowserTool::new(Arc::new(SecurityPolicy::default()), client, 3);
    let error = tool
        .run(
            &json!({"action":"task","goal":"Read the news","flow":{"steps":"not a list"}}),
            None,
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Invalid flow"), "{error}");
}

#[test]
fn a_task_that_found_no_chrome_tells_the_agent_to_ask_for_its_path() {
    let failed = |reason: &str| -> TaskView {
        serde_json::from_value(json!({
            "id": "t-1",
            "status": {"state": "failed", "step": null, "reason": reason,
                       "hint": "", "recoverable": true},
            "summary": "failed",
            "progress": 0.0,
            "next": []
        }))
        .unwrap()
    };
    let hint = host_hint(&failed("browser unavailable: Chrome not found. Checked: …"));
    assert_eq!(hint, Some(crate::modules::browser::CHROME_NOT_FOUND_HINT));
    assert_eq!(
        host_hint(&failed("the last three actions changed nothing")),
        None
    );
    // TinyComputer 0.11's own words: the step's note names a missing Chrome,
    // or a Chrome path that names no Chrome it can start.
    assert_eq!(
        host_hint(&failed(
            "https://www.amazon.in could not be opened: BROWSER_UNAVAILABLE (browser unavailable: no Chrome or Chromium was found on this machine; give the path of the browser to use)"
        )),
        Some(crate::modules::browser::CHROME_NOT_FOUND_HINT)
    );
    assert_eq!(
        host_hint(&failed(
            "https://www.amazon.in could not be opened: BROWSER_UNAVAILABLE (browser unavailable: the browser binary given could not be started (Chrome exited before providing DevTools URL); check that its path names Chrome or Chromium)"
        )),
        Some(crate::modules::browser::CHROME_PATH_HINT)
    );
}

struct ThreadOnly(&'static str);

impl tinytools::ToolRunContext for ThreadOnly {
    fn thread_id(&self) -> Option<&str> {
        Some(self.0)
    }
}

#[tokio::test]
async fn two_agents_on_one_thread_get_distinct_browser_session_keys() {
    use crate::core::runtime::agent_scope::test_agent_context;
    use crate::core::runtime::{context::CoreContext, DomainSet};
    let root = CoreContext::for_test(DomainSet::full(), None);
    let client = Arc::new(BrowserClient::new(Arc::new(
        crate::config::Config::default(),
    )));
    let key_under = |ctx: Arc<CoreContext>| {
        let client = client.clone();
        async move {
            CoreContext::scope(ctx, async move {
                let tool = BrowserTool::new(Arc::new(SecurityPolicy::default()), client, 3);
                let _ = tool
                    .execute_with_context(
                        json!({}),
                        ToolCallOptions::default(),
                        Some(&ThreadOnly("browser-shared-thread")),
                    )
                    .await;
                let key = tool.thread_key.lock().unwrap().clone();
                drop(tool);
                key
            })
            .await
        }
    };

    let alpha = key_under(test_agent_context(&root, "alpha")).await;
    let beta = key_under(test_agent_context(&root, "beta")).await;

    assert!(alpha.is_some() && beta.is_some());
    assert_ne!(alpha, beta);
}
