//! Unit tests for the `automate` loop. A scripted [`AutomateBackend`] feeds
//! canned model responses and records every action, so the loop is exercised
//! with no mic, no AX tree, and no LLM.

use super::*;
use crate::openhuman::accessibility::vision_click::CaptureGeometry;
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
    /// What `frontmost_app` returns (the §1.8 guard input). `None` = unknown.
    frontmost: Option<String>,
    /// What `locate` returns: `Some` screen coords = found; `None` = not found.
    locate_coord: Option<(i32, i32)>,
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
            frontmost: None,
            locate_coord: None,
        }
    }
    fn acts(&self) -> Vec<String> {
        self.acts.lock().unwrap().clone()
    }
    /// Set the frontmost app name reported to the `vision_click` guard.
    fn with_frontmost(mut self, app: &str) -> Self {
        self.frontmost = Some(app.to_string());
        self
    }
    /// Make `locate` report the element found at the given screen coords.
    fn with_located(mut self, x: i32, y: i32) -> Self {
        self.locate_coord = Some((x, y));
        self
    }
}

/// A throwaway geometry for the scripted `screenshot` — tests override `locate`
/// directly, so the transform isn't exercised here (it has its own unit tests).
fn dummy_geom() -> CaptureGeometry {
    CaptureGeometry {
        rect_x: 0,
        rect_y: 0,
        rect_w_pts: 1,
        rect_h_pts: 1,
        img_w_px: 1,
        img_h_px: 1,
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
    async fn screenshot(&self, app: &str) -> Result<(String, CaptureGeometry), String> {
        self.acts.lock().unwrap().push(format!("screenshot:{app}"));
        Ok(("data:image/png;base64,TEST".to_string(), dummy_geom()))
    }
    async fn locate(
        &self,
        _shot: &str,
        _geom: &CaptureGeometry,
        description: &str,
    ) -> Result<Option<(i32, i32)>, String> {
        self.acts
            .lock()
            .unwrap()
            .push(format!("locate:{description}"));
        Ok(self.locate_coord)
    }
    async fn frontmost_app(&self) -> Option<String> {
        self.frontmost.clone()
    }
    async fn click(&self, x: i32, y: i32) -> Result<String, String> {
        self.acts.lock().unwrap().push(format!("click:{x},{y}"));
        Ok(format!("Clicked at ({x}, {y})"))
    }
    async fn open_url(&self, url: &str) -> Result<String, String> {
        self.acts.lock().unwrap().push(format!("open_url:{url}"));
        Ok(format!("Opened {url}"))
    }
    async fn key(&self, keys: &[String]) -> Result<String, String> {
        self.acts
            .lock()
            .unwrap()
            .push(format!("key:{}", keys.join("+")));
        Ok(format!("Executed {}", keys.join("+")))
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
async fn hotkey_verb_sends_chord_to_backend() {
    // The model can drive an app shortcut via the general loop instead of
    // hunting AX labels. Use a neutral app/goal so no fast-path intercepts and
    // the loop's `hotkey` verb is what runs.
    let backend = ScriptedBackend::new(&[
        r#"{"action":"hotkey","keys":["Cmd","L"]}"#,
        r#"{"action":"done","summary":"sent shortcut"}"#,
    ]);
    let out = run("Notes", "do a thing", &backend, opts(5)).await;
    assert!(out.success, "expected success, got {out:?}");
    assert_eq!(
        backend.acts(),
        vec!["launch:Notes", "key:Cmd+L"] // foreground-first, then the chord
    );
}

#[tokio::test]
async fn budget_exhaustion_fails() {
    // Script always lists → never done → budget guard ends the run.
    let backend = ScriptedBackend::new(&[r#"{"action":"list","filter":"x"}"#]);
    let out = run("Music", "never finishes", &backend, opts(3)).await;
    assert!(!out.success);
    assert!(out.summary.contains("budget"), "got: {}", out.summary);
    // #2: surfaces what was on screen + a "try a different approach" steer.
    assert!(out.summary.contains("On screen:"), "got: {}", out.summary);
    assert!(
        out.summary.to_lowercase().contains("different approach"),
        "got: {}",
        out.summary
    );
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
        out.summary.to_lowercase().contains("stuck repeating"),
        "got: {}",
        out.summary
    );
    // #1/#2: actionable — names the screen and tells it to switch tactics.
    assert!(out.summary.contains("On screen:"), "got: {}", out.summary);
    assert!(
        out.summary.to_lowercase().contains("switch tactics"),
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

// ── vision_click fallback ────────────────────────────────────────────────────

#[tokio::test]
async fn vision_click_locates_and_clicks_when_frontmost() {
    let backend = ScriptedBackend::new(&[
        r#"{"action":"vision_click","description":"the Call button"}"#,
        r#"{"action":"done","summary":"clicked"}"#,
    ])
    .with_frontmost("Slack")
    .with_located(640, 360);
    let out = run("Slack", "click the call button", &backend, opts(5)).await;
    assert!(out.success, "{out:?}");
    let acts = backend.acts();
    assert!(acts.contains(&"screenshot:Slack".to_string()), "{acts:?}");
    assert!(
        acts.contains(&"locate:the Call button".to_string()),
        "{acts:?}"
    );
    assert!(acts.contains(&"click:640,360".to_string()), "{acts:?}");
}

#[tokio::test]
async fn vision_click_proceeds_when_frontmost_unknown() {
    // `None` frontmost (e.g. can't determine) is best-effort: the loop already
    // foregrounded the app, so we still click.
    let backend = ScriptedBackend::new(&[
        r#"{"action":"vision_click","description":"X"}"#,
        r#"{"action":"done"}"#,
    ])
    .with_located(10, 20);
    let out = run("Slack", "x", &backend, opts(5)).await;
    assert!(out.success);
    assert!(backend.acts().contains(&"click:10,20".to_string()));
}

#[tokio::test]
async fn vision_click_refused_when_other_app_frontmost() {
    // Positive evidence a different app is focused → refuse (the §1.8 guard).
    let backend = ScriptedBackend::new(&[
        r#"{"action":"vision_click","description":"the Call button"}"#,
        r#"{"action":"done","summary":"done"}"#,
    ])
    .with_frontmost("Finder")
    .with_located(640, 360);
    let out = run("Slack", "click call", &backend, opts(5)).await;
    let acts = backend.acts();
    assert!(
        !acts.iter().any(|a| a.starts_with("click:")),
        "must not click into a non-target app: {acts:?}"
    );
    assert!(
        !acts.iter().any(|a| a.starts_with("screenshot:")),
        "must not even screenshot when refused: {acts:?}"
    );
    let _ = out;
}

#[tokio::test]
async fn vision_click_not_found_does_not_click() {
    let backend = ScriptedBackend::new(&[
        r#"{"action":"vision_click","description":"the Call button"}"#,
        r#"{"action":"done","summary":"gave up"}"#,
    ])
    .with_frontmost("Slack"); // locate_coord stays None → not found
    let out = run("Slack", "click call", &backend, opts(5)).await;
    assert!(out.success);
    let acts = backend.acts();
    assert!(acts.contains(&"screenshot:Slack".to_string()), "{acts:?}");
    assert!(
        acts.contains(&"locate:the Call button".to_string()),
        "{acts:?}"
    );
    assert!(
        !acts.iter().any(|a| a.starts_with("click:")),
        "no click when the element isn't found: {acts:?}"
    );
}

#[tokio::test]
async fn vision_click_empty_description_skipped() {
    let backend = ScriptedBackend::new(&[
        r#"{"action":"vision_click","description":"  "}"#,
        r#"{"action":"done"}"#,
    ])
    .with_frontmost("Slack");
    let out = run("Slack", "x", &backend, opts(5)).await;
    assert!(out.success);
    assert!(
        !backend.acts().iter().any(|a| a.starts_with("screenshot:")),
        "empty description must be skipped before any capture"
    );
}
