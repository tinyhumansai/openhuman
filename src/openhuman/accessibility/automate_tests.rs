//! Unit tests for the `automate` loop. A scripted [`AutomateBackend`] feeds
//! canned model responses and records every action, so the loop is exercised
//! with no mic, no AX tree, and no LLM.

use super::*;
use std::sync::Mutex;

/// Scripted backend: `decide` returns the next queued response each call;
/// perceive/act are stubbed and recorded.
struct ScriptedBackend {
    /// Queued raw model outputs, consumed in order.
    responses: Mutex<std::collections::VecDeque<String>>,
    /// Elements every `perceive` returns.
    elements: Vec<ax::AXElement>,
    /// Record of act calls, for assertions.
    acts: Mutex<Vec<String>>,
    /// Force act_press to error (to exercise the failure-recording path).
    press_errors: bool,
}

impl ScriptedBackend {
    fn new(responses: &[&str]) -> Self {
        Self {
            responses: Mutex::new(responses.iter().map(|s| s.to_string()).collect()),
            elements: vec![
                ax::AXElement::new("AXButton", "Play"),
                ax::AXElement::new("AXTextField", "Search"),
            ],
            acts: Mutex::new(Vec::new()),
            press_errors: false,
        }
    }
    fn acts(&self) -> Vec<String> {
        self.acts.lock().unwrap().clone()
    }
}

#[async_trait]
impl AutomateBackend for ScriptedBackend {
    async fn perceive(&self, _app: &str, _filter: &str) -> Result<Vec<ax::AXElement>, String> {
        Ok(self.elements.clone())
    }
    async fn decide(&self, _system: &str, _user: &str) -> Result<String, String> {
        Ok(self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            // When the script runs dry, keep listing so the budget guard is what
            // ends the run (rather than a decide error).
            .unwrap_or_else(|| r#"{"action":"list","filter":""}"#.to_string()))
    }
    async fn act_launch(&self, app: &str) -> Result<String, String> {
        self.acts.lock().unwrap().push(format!("launch:{app}"));
        Ok(format!("Opened '{app}'."))
    }
    async fn act_press(&self, app: &str, label: &str) -> Result<String, String> {
        self.acts
            .lock()
            .unwrap()
            .push(format!("press:{app}:{label}"));
        if self.press_errors {
            return Err("no such element".into());
        }
        Ok(format!("Pressed '{label}' in '{app}'."))
    }
    async fn act_set_value(&self, app: &str, label: &str, value: &str) -> Result<String, String> {
        self.acts
            .lock()
            .unwrap()
            .push(format!("set_value:{app}:{label}={value}"));
        Ok(format!("Set '{label}' in '{app}'."))
    }
    async fn open_url(&self, url: &str) -> Result<String, String> {
        self.acts.lock().unwrap().push(format!("open_url:{url}"));
        Ok(format!("Opened {url}"))
    }
    async fn settle(&self, _app: &str) {}
    async fn wait(&self, _ms: u64) {}
}

fn opts(budget: u32) -> AutomateOptions {
    AutomateOptions {
        step_budget: budget,
    }
}

#[tokio::test]
async fn happy_path_launch_list_press_done() {
    // Use a non-fast-path app/goal so the GENERAL loop is what runs.
    // run() foregrounds (launch) the app first, so the model needn't.
    let backend = ScriptedBackend::new(&[
        r#"{"action":"list","filter":"Play"}"#,
        r#"{"action":"press","label":"Play"}"#,
        r#"{"action":"done","summary":"Playing."}"#,
    ]);
    let out = run("Notes", "do a thing", &backend, opts(8)).await;
    assert!(out.success, "expected success, got {out:?}");
    assert_eq!(out.summary, "Playing.");
    let acts = backend.acts();
    // Leading launch is the foreground-first guarantee.
    assert_eq!(acts, vec!["launch:Notes", "press:Notes:Play"]);
}

#[tokio::test]
async fn navigate_then_activate_sequence() {
    // Press the row (navigates), then press the detail Play, then done.
    // Non-fast-path app so this exercises the general loop's two-press flow.
    let backend = ScriptedBackend::new(&[
        r#"{"action":"press","label":"Highway to Hell"}"#,
        r#"{"action":"press","label":"Play"}"#,
        r#"{"action":"done","summary":"ok"}"#,
    ]);
    let out = run("Photos", "open the top album", &backend, opts(8)).await;
    assert!(out.success);
    assert_eq!(
        backend.acts(),
        vec![
            "launch:Photos", // foreground-first
            "press:Photos:Highway to Hell",
            "press:Photos:Play"
        ]
    );
}

#[tokio::test]
async fn set_value_routes_app_override() {
    let backend = ScriptedBackend::new(&[
        r#"{"action":"set_value","app":"Slack","label":"message","value":"hi"}"#,
        r#"{"action":"done"}"#,
    ]);
    let out = run("Slack", "message Steven hi", &backend, opts(5)).await;
    assert!(out.success);
    assert_eq!(
        backend.acts(),
        vec!["launch:Slack", "set_value:Slack:message=hi"] // foreground-first
    );
}

#[tokio::test]
async fn budget_exhaustion_fails() {
    // Script always lists → never done → budget guard ends the run.
    let backend = ScriptedBackend::new(&[r#"{"action":"list","filter":"x"}"#]);
    let out = run("Music", "never finishes", &backend, opts(3)).await;
    assert!(!out.success);
    assert!(out.summary.contains("budget"), "got: {}", out.summary);
}

#[tokio::test]
async fn no_progress_guard_aborts_repeated_action() {
    // Model keeps pressing the same control (the live "Search ×11" pathology).
    let backend = ScriptedBackend::new(&[
        r#"{"action":"press","label":"Search"}"#,
        r#"{"action":"press","label":"Search"}"#,
        r#"{"action":"press","label":"Search"}"#,
        r#"{"action":"press","label":"Search"}"#,
    ]);
    let out = run("Photos", "do something", &backend, opts(10)).await;
    assert!(!out.success);
    assert!(
        out.summary.contains("stuck repeating"),
        "got: {}",
        out.summary
    );
    // foreground launch, then acted twice; the 3rd identical action aborts.
    assert_eq!(
        backend.acts(),
        vec![
            "launch:Photos",
            "press:Photos:Search",
            "press:Photos:Search"
        ]
    );
}

#[tokio::test]
async fn one_repair_retry_then_succeeds() {
    let backend = ScriptedBackend::new(&[
        "garbage not json",
        r#"{"action":"done","summary":"recovered"}"#,
    ]);
    let out = run("Music", "g", &backend, opts(5)).await;
    assert!(out.success, "should recover after one repair: {out:?}");
    assert_eq!(out.summary, "recovered");
}

#[tokio::test]
async fn two_unparseable_outputs_fail() {
    let backend = ScriptedBackend::new(&["garbage one", "garbage two"]);
    let out = run("Music", "g", &backend, opts(5)).await;
    assert!(!out.success);
    assert!(out.summary.contains("unparseable"), "got: {}", out.summary);
}

#[tokio::test]
async fn explicit_fail_action_propagates() {
    let backend = ScriptedBackend::new(&[r#"{"action":"fail","summary":"app not installed"}"#]);
    let out = run("Music", "x", &backend, opts(5)).await;
    assert!(!out.success);
    assert_eq!(out.summary, "app not installed");
}

#[tokio::test]
async fn press_failure_is_recorded_not_fatal() {
    let mut backend = ScriptedBackend::new(&[
        r#"{"action":"press","label":"Play"}"#,
        r#"{"action":"done","summary":"tried"}"#,
    ]);
    backend.press_errors = true;
    let out = run("Music", "x", &backend, opts(5)).await;
    assert!(out.success); // the run continues; the press failure is just logged
    assert!(
        out.steps.iter().any(|s| s.contains("press FAILED")),
        "steps: {:?}",
        out.steps
    );
}

#[test]
fn parse_action_plain_json() {
    let a = parse_action(r#"{"action":"press","label":"Play"}"#).unwrap();
    assert_eq!(a.action, "press");
    assert_eq!(a.label, "Play");
}

#[test]
fn parse_action_strips_code_fence_and_prose() {
    let raw = "Sure!\n```json\n{\"action\":\"done\",\"summary\":\"ok\"}\n```\n";
    let a = parse_action(raw).unwrap();
    assert_eq!(a.action, "done");
    assert_eq!(a.summary, "ok");
}

#[test]
fn parse_action_rejects_garbage() {
    assert!(parse_action("not json at all").is_err());
    assert!(parse_action("").is_err());
}

#[test]
fn render_snapshot_caps_and_labels() {
    let many: Vec<ax::AXElement> = (0..100)
        .map(|i| ax::AXElement::new("AXButton", format!("btn{i}")))
        .collect();
    let s = render_snapshot("Music", "btn", &many);
    assert!(s.contains("showing 40 of 100"));
    assert!(s.contains("btn0"));
    assert!(!s.contains("btn50"), "should be capped at 40");
}

#[test]
fn render_snapshot_does_not_annotate_enabled() {
    // AXEnabled is unreliable per-app, so the snapshot must not surface it
    // (would mislead the model into avoiding pressable controls).
    let mut disabled = ax::AXElement::new("AXButton", "Play");
    disabled.enabled = Some(false);
    let s = render_snapshot("Music", "", &[disabled]);
    assert!(!s.contains("disabled"), "got: {s}");
    assert!(s.contains("[AXButton] Play"));
}

#[test]
fn render_snapshot_empty_hint() {
    let s = render_snapshot("Music", "zzz", &[]);
    assert!(s.contains("no elements"));
}
