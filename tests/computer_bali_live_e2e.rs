//! Live end-to-end test: book a trip to Bali through TinyComputer, up to the
//! payment page, exactly the way the agent's `browser` tool does it.
//!
//! This is the OpenHuman counterpart of TinyComputer's Kashmir demo. It goes
//! through the real host path, not a stub:
//!
//! 1. the checksum-pinned TinyComputer release is downloaded, admitted, and
//!    loaded through the module registry (`modules::desktop::proxy`);
//! 2. the host hands it its private configuration — the decision model
//!    (`[computer] decision_model`, Jev over OpenRouter here), and the planner
//!    and rescue route (`modules::computer_config`);
//! 3. `modules::browser_task` starts the task with `StartTask` (browser only,
//!    confined to the allowed websites, never allowed to pay), follows it with
//!    `AwaitTask`, answers `needs_input` from the facts, approves ordinary
//!    booking steps, and refuses any approval that would pay.
//!
//! It never pays: TinyComputer always stops at a payment page, and this test
//! treats reaching that checkpoint as success.
//!
//! Run manually, on a machine with Google Chrome, network access and an
//! OpenRouter key (the browser window is shown while it works):
//!
//! ```sh
//! OPENROUTER_API_KEY=... cargo test -p openhuman-cli --test computer_bali_live_e2e \
//!   -- --ignored --nocapture
//! ```
//!
//! Optional: `COMPUTER_E2E_CHROME` (Chrome executable), `COMPUTER_E2E_HEADLESS=1`,
//! `COMPUTER_E2E_TASK_FILE` / `COMPUTER_E2E_FACTS_FILE` / `COMPUTER_E2E_FLOW_FILE`
//! (replay another saved task, e.g. TinyComputer's Kashmir demo),
//! `COMPUTER_E2E_MINUTES` (default 20), `COMPUTER_E2E_PLAN=1` (plan instead of
//! replaying `fixtures/computer/bali/plan.json`), `COMPUTER_E2E_DECISION_MODEL`
//! (`jev`, `open_jev`, `sage`), `COMPUTER_E2E_OUT` (report directory).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use openhuman_core::config::{Config, DecisionModel};
use openhuman_core::modules::browser_task::{self, BrowserTask};
use tinycomputer_bus::agent::{ContinueTaskRequest, TaskStatus, TaskView};

const TASK: &str = include_str!("fixtures/computer/bali/task.md");
const FACTS: &str = include_str!("fixtures/computer/bali/facts.json");
/// The flow to replay, adapted from the Kashmir demo's recorded plan. Replaying
/// a saved flow is how that demo's passing runs were made; set
/// `COMPUTER_E2E_PLAN=1` to let the planner write a fresh one instead.
const PLAN: &str = include_str!("fixtures/computer/bali/plan.json");
const DEFAULT_CHROME: &str = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";

fn config(workspace: &std::path::Path) -> Config {
    let mut config = Config {
        config_path: workspace.join("config.toml"),
        workspace_dir: workspace.join("workspace"),
        ..Default::default()
    };
    config.secrets.encrypt = false;
    config.modules.enabled = true;
    config.browser.enabled = true;
    config.browser.headless = std::env::var("COMPUTER_E2E_HEADLESS").is_ok_and(|v| v == "1");
    config.browser.task_timeout_secs = 600;
    let chrome = std::env::var("COMPUTER_E2E_CHROME").unwrap_or_else(|_| DEFAULT_CHROME.into());
    if std::path::Path::new(&chrome).exists() {
        config.browser.chrome_path = Some(chrome);
    }
    config.http_request.allowed_domains = vec!["google.com".into(), "goindigo.in".into()];
    config.computer.decision_model = match std::env::var("COMPUTER_E2E_DECISION_MODEL").as_deref() {
        Ok("open_jev") => DecisionModel::OpenJev,
        Ok("sage") => DecisionModel::Sage,
        _ => DecisionModel::Jev,
    };
    config
}

/// Print what the task did and every rescue, so a failed live run says why.
async fn print_report(config: &Config, view: &TaskView) {
    match browser_task::report(config, view.id.clone()).await {
        Ok(report) => {
            for (index, step) in report.steps.iter().enumerate() {
                println!(
                    "  step {index}: {}",
                    serde_json::to_string(step).unwrap_or_default()
                );
            }
            for rescue in &report.rescues {
                println!(
                    "  rescue of step {}: {:?} — {} ({})",
                    rescue.step, rescue.outcome, rescue.reason, rescue.failure
                );
            }
            if report.rescues.is_empty() {
                println!("  no rescues");
            }
        }
        Err(error) => println!("  report unavailable: {error}"),
    }
}

fn summarize(view: &TaskView) -> String {
    let step = view
        .step
        .as_ref()
        .map(|step| {
            format!(
                " step {}/{} {} ({})",
                step.index + 1,
                step.total,
                step.kind,
                step.intent
            )
        })
        .unwrap_or_default();
    format!("[{:>3.0}%]{step} — {}", view.progress * 100.0, view.summary)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "live: downloads the tinycomputer module, launches Chrome and calls OpenRouter; needs OPENROUTER_API_KEY. Run: cargo test -p openhuman-cli --test computer_bali_live_e2e -- --ignored --nocapture"]
async fn books_a_bali_flight_up_to_the_payment_page() {
    if std::env::var("OPENROUTER_API_KEY").map_or(true, |key| key.trim().is_empty()) {
        eprintln!("skipping: OPENROUTER_API_KEY is not set");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let config = config(dir.path());
    // Overrides let the same harness replay another saved task, such as
    // TinyComputer's own Kashmir demo, to tell site drift from host bugs.
    let read = |var: &str, default: &str| {
        std::env::var(var).map_or_else(
            |_| default.to_owned(),
            |path| std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{var}={path}: {e}")),
        )
    };
    let task_text = read("COMPUTER_E2E_TASK_FILE", TASK);
    let facts_text = read("COMPUTER_E2E_FACTS_FILE", FACTS);
    let plan_text = read("COMPUTER_E2E_FLOW_FILE", PLAN);
    let facts: BTreeMap<String, String> = serde_json::from_str(&facts_text).expect("facts");

    let status = openhuman_core::modules::computer::status(&config, true).await;
    println!(
        "module: {:?} v{} decision={} ({}) planner={}",
        status.module.as_ref().map(|m| &m.state),
        status.module.as_ref().map_or("?", |m| m.version.as_str()),
        status.decision_model,
        status.decision_route,
        status.planner_route
    );
    let capabilities = status
        .capabilities
        .unwrap_or_else(|| panic!("TinyComputer did not describe itself: {:?}", status.error));
    assert!(
        capabilities.compatible,
        "contract {:?}",
        capabilities.contract_version
    );
    assert!(capabilities.jev_configured, "no decision model configured");
    assert!(capabilities.planner_configured, "no planner configured");
    assert!(capabilities.rescue_configured, "no rescue model configured");

    let task = BrowserTask {
        goal: task_text,
        facts: facts.clone(),
        origins: vec!["https://.google.com".into(), "https://.goindigo.in".into()],
        max_actions: 200,
        flow: if std::env::var("COMPUTER_E2E_PLAN").is_ok_and(|v| v == "1") {
            None
        } else {
            Some(serde_json::from_str(&plan_text).expect("saved flow"))
        },
        site: None,
    };
    let minutes = std::env::var("COMPUTER_E2E_MINUTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20u64);
    let deadline = Instant::now() + Duration::from_secs(minutes * 60);

    let mut view = browser_task::start(&config, &task)
        .await
        .expect("StartTask");
    println!("task {} started", view.id);
    loop {
        println!("{}", summarize(&view));
        match &view.status {
            TaskStatus::Running => {
                assert!(
                    Instant::now() < deadline,
                    "task still running after {minutes} min"
                );
                view = browser_task::wait(&config, view.id.clone())
                    .await
                    .expect("AwaitTask");
            }
            TaskStatus::NeedsInput { fields } => {
                let inputs: BTreeMap<String, String> = fields
                    .iter()
                    .filter_map(|field| {
                        facts
                            .get(&field.name)
                            .map(|v| (field.name.clone(), v.clone()))
                    })
                    .collect();
                assert!(
                    !inputs.is_empty(),
                    "task asked for values the facts do not have: {:?}",
                    fields.iter().map(|f| &f.name).collect::<Vec<_>>()
                );
                view = browser_task::resume(
                    &config,
                    ContinueTaskRequest {
                        id: view.id.clone(),
                        inputs,
                        ..ContinueTaskRequest::default()
                    },
                )
                .await
                .expect("ContinueTask inputs");
            }
            TaskStatus::NeedsApproval { action, target, .. } => {
                // A host approves ordinary booking steps (opening a tab,
                // continuing a form) and never one that pays. TinyComputer also
                // stops at the payment page on its own.
                let text = format!("{action} {target}").to_lowercase();
                let pays = ["pay", "purchase", "buy", "card", "checkout"]
                    .iter()
                    .any(|word| text.contains(word));
                println!(
                    "{} approval: {action} ({target})",
                    if pays { "refusing" } else { "granting" }
                );
                view = browser_task::resume(
                    &config,
                    ContinueTaskRequest {
                        id: view.id.clone(),
                        approve: Some(!pays),
                        ..ContinueTaskRequest::default()
                    },
                )
                .await
                .expect("ContinueTask approval");
            }
            TaskStatus::Checkpoint {
                reason,
                location,
                summary,
                ..
            } => {
                println!("checkpoint at {location}: {reason}\n{summary}");
                print_report(&config, &view).await;
                assert!(
                    location.contains("goindigo") || reason.to_lowercase().contains("pay"),
                    "stopped at an unexpected checkpoint: {reason} ({location})"
                );
                break;
            }
            TaskStatus::Done { answer, .. } => {
                println!("done: {answer}");
                break;
            }
            TaskStatus::NeedsHuman { reason, .. } => {
                let _ = browser_task::cancel(&config, view.id.clone()).await;
                panic!("task needs a person: {reason}");
            }
            TaskStatus::NeedsPlan { .. } => panic!("the planner is not configured"),
            TaskStatus::Failed { reason, hint, .. } => {
                print_report(&config, &view).await;
                panic!("task failed: {reason} ({hint})")
            }
            TaskStatus::Cancelled => panic!("task was cancelled"),
        }
    }
}
