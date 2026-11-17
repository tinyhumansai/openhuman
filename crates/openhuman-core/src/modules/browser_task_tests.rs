use super::*;
use serde_json::json;
use tinycomputer_bus::agent::AgentError;

fn task() -> BrowserTask {
    BrowserTask {
        goal: "Find the opening hours".into(),
        facts: BTreeMap::from([("name".into(), "Asha".into())]),
        origins: vec!["https://.example.com".into()],
        max_actions: 12,
        flow: None,
        site: None,
    }
}

#[test]
fn start_request_confines_the_task_to_the_browser_under_host_policy() {
    let mut config = Config::default();
    config.browser.headless = false;
    config.browser.task_timeout_secs = 90;
    config.computer.max_rescues = Some(2);
    let request = start_request(&config, &task());
    assert_eq!(request.task.as_deref(), Some("Find the opening hours"));
    assert_eq!(request.facts["name"], "Asha");
    assert_eq!(request.constraints.surfaces, vec![SurfaceKind::Browser]);
    assert_eq!(request.constraints.origins, vec!["https://.example.com"]);
    assert_eq!(request.constraints.payment, PaymentMode::StopAtPayment);
    assert!(!request.constraints.allow_destructive);
    assert!(request.constraints.headed);
    assert!(request.constraints.browser_executable.is_none());
    assert!(request.constraints.browser_profile.is_none());
    assert_eq!(request.budget.max_actions, Some(12));
    assert_eq!(request.budget.max_elapsed_ms, Some(90_000));
    assert_eq!(request.budget.max_rescues, Some(2));
}

#[test]
fn a_task_carries_the_chrome_path_and_the_persistent_profile_from_settings() {
    let mut config = Config::default();
    config.browser.chrome_path =
        Some("/Users/asha/Desktop/Google Chrome.app/Contents/MacOS/Google Chrome".into());
    config.browser.profile_mode = "persistent".into();
    config.browser.profile_path = Some("/Users/asha/.openhuman/chrome".into());
    let request = start_request(&config, &task());
    assert_eq!(
        request.constraints.browser_executable.as_deref(),
        Some("/Users/asha/Desktop/Google Chrome.app/Contents/MacOS/Google Chrome")
    );
    assert_eq!(
        request.constraints.browser_profile.as_deref(),
        Some("/Users/asha/.openhuman/chrome")
    );
    // A fresh profile is the module's own temporary one.
    config.browser.profile_mode = "fresh".into();
    assert!(start_request(&config, &task())
        .constraints
        .browser_profile
        .is_none());
    assert!(start_request(&Config::default(), &task())
        .constraints
        .browser_executable
        .is_none());
}

#[test]
fn a_traced_host_asks_the_module_to_record_every_decision() {
    let mut config = Config::default();
    config.computer.trace = true;
    assert!(start_request(&config, &task()).trace);
}

#[tokio::test]
async fn start_refuses_a_task_without_origins() {
    let mut empty = task();
    empty.origins.clear();
    let error = start(&Config::default(), &empty).await.unwrap_err();
    assert!(error.contains("allowed origin"), "{error}");
}

#[test]
fn the_module_is_told_the_site_only_while_learning_is_on() {
    let workspace = tempfile::tempdir().unwrap();
    let mut config = Config {
        workspace_dir: workspace.path().to_path_buf(),
        ..Config::default()
    };
    config.browser.learn_from_tasks = true;
    let mut on_a_site = task();
    on_a_site.site = Some("shop.test".to_owned());

    assert_eq!(
        start_request(&config, &on_a_site).site.as_deref(),
        Some("shop.test")
    );
    // A switch in Browser settings outranks the config a tool was built with.
    super::super::site_memory_host::note_switch(workspace.path(), false);
    assert_eq!(start_request(&config, &on_a_site).site, None);
    super::super::site_memory_host::note_switch(workspace.path(), true);
    assert_eq!(
        start_request(&config, &on_a_site).site.as_deref(),
        Some("shop.test")
    );
    // A task that names no site hands the module none.
    assert_eq!(start_request(&config, &task()).site, None);
}

#[test]
fn unwrap_response_returns_data_or_the_structured_error() {
    let ok = AgentResponse {
        version: tinycomputer_bus::agent::AGENT_ENVELOPE_VERSION.to_owned(),
        ok: true,
        data: Some(7),
        error: None,
    };
    assert_eq!(unwrap_response("StartTask", ok), Ok(7));
    let failed: AgentResponse<u32> = AgentResponse {
        version: tinycomputer_bus::agent::AGENT_ENVELOPE_VERSION.to_owned(),
        ok: false,
        data: None,
        error: Some(AgentError::new(
            "PLANNER_UNAVAILABLE",
            "no planner",
            "configure one",
            false,
        )),
    };
    let error = unwrap_response("StartTask", failed).unwrap_err();
    assert!(error.contains("PLANNER_UNAVAILABLE"), "{error}");
    let empty: AgentResponse<u32> = AgentResponse {
        version: tinycomputer_bus::agent::AGENT_ENVELOPE_VERSION.to_owned(),
        ok: true,
        data: None,
        error: None,
    };
    assert!(unwrap_response("AwaitTask", empty)
        .unwrap_err()
        .contains("no result"));
}

#[test]
fn a_chrome_path_the_module_refuses_says_where_the_user_fixes_it() {
    // TinyComputer refuses a task whose Chrome path names no program in its
    // own words; the agent is told where the user sets that path.
    let refused: AgentResponse<u32> = AgentResponse {
        version: tinycomputer_bus::agent::AGENT_ENVELOPE_VERSION.to_owned(),
        ok: false,
        data: None,
        error: Some(AgentError::new(
            "INVALID_REQUEST",
            "browser_executable must be the absolute path of a browser binary on this machine",
            "",
            false,
        )),
    };
    let error = unwrap_response("StartTask", refused).unwrap_err();
    assert!(
        error.starts_with(
            "browser task StartTask failed [INVALID_REQUEST]: browser_executable must be"
        ),
        "{error}"
    );
    assert!(
        error.ends_with(crate::modules::browser::CHROME_PATH_HINT),
        "{error}"
    );
}

#[tokio::test]
async fn a_task_without_a_persistent_profile_claims_nothing() {
    let config = Config::default();
    let mut request = start_request(&config, &task());
    assert_eq!(claim_profile(&config, &mut request, 50).await, None);
    assert!(request.constraints.browser_profile.is_none());
}

#[test]
fn a_saved_flow_is_run_instead_of_planned() {
    let flow: tinycomputer_bus::Flow = serde_json::from_value(serde_json::json!({
        "app": "browser",
        "steps": [{"browse": "https://www.example.com/"}, {"stop_before": "paying"}]
    }))
    .unwrap();
    let mut saved = task();
    saved.flow = Some(flow.clone());
    let request = start_request(&Config::default(), &saved);
    assert_eq!(request.flow, Some(flow));
    assert_eq!(request.task.as_deref(), Some("Find the opening hours"));
}

#[tokio::test]
async fn a_task_that_already_stopped_settles_without_calling_the_module() {
    // Not running, so no `AwaitTask`; finished and untraced, so no report is
    // fetched either.
    let mut config = Config::default();
    config.computer.trace = false;
    let view: TaskView = serde_json::from_value(json!({
        "id": "t-settled",
        "status": {"state": "done", "answer": "found", "records": {}},
        "summary": "found it",
        "progress": 1.0,
        "next": []
    }))
    .unwrap();
    let settled = settle(&config, view.clone()).await.unwrap();
    assert_eq!(settled, view);
}
