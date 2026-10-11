use super::*;

use std::sync::Arc;

use tinyagents_harness::context::{RunConfig, RunContext};
use tinyagents_harness::limits::RunLimits;
use tinyagents_harness::middleware::Middleware;
use tinyagents_harness::runtime::{AgentHarness, RunPolicy};
use tinyagents_harness::testkit::{FakeTool, ScriptedModel};
use tinyagents_harness::tinyinference_llm::model::ModelRequest;
use tinyagents_harness::tinyinference_llm::tool::ToolCall;

/// The real request this middleware was built for names three inputs and one
/// output. Extraction deliberately cannot tell them apart -- that is the
/// existence check's job -- so it must return all four.
const ATRX_REQUEST: &str = "Catalogue all coding variants present in the mutated ATRX \
    transcripts at /app/data/mutated-transcripts.txt relative to the wild-type NM_000489.6 \
    reference (encoded by /app/data/genomic-locus.fa and the CDS information at \
    /app/data/CDS-information.txt). Write the final results to \
    /app/output/mutation.report.json as a single JSON object.";

#[test]
fn extraction_returns_every_named_file_input_and_output_alike() {
    assert_eq!(
        candidate_paths(ATRX_REQUEST),
        vec![
            "/app/data/mutated-transcripts.txt",
            "/app/data/genomic-locus.fa",
            "/app/data/CDS-information.txt",
            "/app/output/mutation.report.json",
        ]
    );
}

/// The design claim: separating a deliverable from an input needs no grammar,
/// because an input is on disk and a skipped deliverable is not.
#[test]
fn existence_alone_separates_a_skipped_deliverable_from_the_inputs() {
    let dir = std::env::temp_dir().join(format!("oh-unmet-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let input = dir.join("given.txt");
    std::fs::write(&input, b"provided").expect("write input");
    let output = dir.join("report.json");
    let _ = std::fs::remove_file(&output);

    let candidates = vec![
        input.to_string_lossy().to_string(),
        output.to_string_lossy().to_string(),
    ];
    assert_eq!(
        UnmetDeliverableMiddleware::missing(&candidates),
        vec![output.to_string_lossy().to_string()],
        "the input must not be reported; only the path nothing created"
    );

    // Once written, even empty, it stops being reported: this middleware
    // checks existence, never contents.
    std::fs::write(&output, b"{}").expect("write output");
    assert!(UnmetDeliverableMiddleware::missing(&candidates).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn only_absolute_paths_carrying_a_file_extension_are_candidates() {
    for rejected in [
        "see config/settings.yml for the rest", // relative: a reference, not a deliverable
        "write it under /app/output/",          // a directory, no extension
        "results go in /report.json",           // single segment at the root
        "read /app/../etc/passwd",              // traversal is never statted
        "the ratio is 3/4.5 overall",           // arithmetic, not a path
        "fetch https://example.com/report.pdf", // a URL, not a local file
        "see http://host:8080/a/b.csv and //cdn/x.js", // scheme-less `//` too
    ] {
        assert!(
            candidate_paths(rejected).is_empty(),
            "must not treat {rejected:?} as a deliverable: {:?}",
            candidate_paths(rejected)
        );
    }
}

#[test]
fn a_path_is_recognised_through_the_punctuation_a_request_wraps_it_in() {
    for (text, expected) in [
        ("write to `/app/out/r.json`.", "/app/out/r.json"),
        ("write to \"/app/out/r.json\",", "/app/out/r.json"),
        ("write to (/app/out/r.json)", "/app/out/r.json"),
        ("write to /app/out/r.json.", "/app/out/r.json"),
        ("write to </app/out/r.json>", "/app/out/r.json"),
    ] {
        assert_eq!(candidate_paths(text), vec![expected], "for {text:?}");
    }
}

#[test]
fn a_repeated_path_is_reported_once_and_a_long_list_is_bounded() {
    let twice = "first /a/b/c.json then /a/b/c.json again";
    assert_eq!(candidate_paths(twice), vec!["/a/b/c.json"]);

    let many = (0..20)
        .map(|i| format!("/dir/file{i}.json"))
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(candidate_paths(&many).len(), MAX_CANDIDATES);
}

#[test]
fn the_notice_names_the_paths_and_asks_for_a_partial_file() {
    let one = notice(&["/app/output/r.json".to_string()]);
    assert!(one.contains("<harness_instruction>"));
    assert!(one.contains("a file that does not exist here"));
    assert!(one.contains("lives elsewhere"));
    assert!(one.contains("`/app/output/r.json`"));
    assert!(one.contains("even where fields are incomplete or provisional"));

    let two = notice(&["/a/x.json".to_string(), "/b/y.csv".to_string()]);
    assert!(two.contains("files that do not exist here"));
    assert!(two.contains("`/a/x.json`, `/b/y.csv`"));
}

fn tool_round(id: &str, name: &str) -> tinyagents_harness::tinyinference_llm::model::ModelResponse {
    let mut response =
        tinyagents_harness::tinyinference_llm::model::ModelResponse::assistant(String::new());
    response.message.content = Vec::new();
    response.message.tool_calls = vec![ToolCall::new(id, name, serde_json::json!({}))];
    response.finish_reason = Some("tool_calls".to_string());
    response
}

/// Drive a run whose request names a path that does not exist. The notice has
/// to reach the model, leave it free to call tools, and let its second answer
/// stand -- and it must not be given twice.
#[tokio::test]
async fn an_unwritten_deliverable_holds_the_answer_once_and_permits_a_fix() {
    let missing = std::env::temp_dir().join(format!("oh-unmet-run-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&missing);
    let request = format!("Do the work and write it to {}", missing.display());

    let mut harness: AgentHarness<()> = AgentHarness::new();
    harness.register_model(
        "mock",
        Arc::new(ScriptedModel::new(vec![
            tool_round("c0", "writer"),
            tinyagents_harness::tinyinference_llm::model::ModelResponse::assistant(
                "here is what I found".to_string(),
            ),
            tool_round("c1", "writer"),
            tinyagents_harness::tinyinference_llm::model::ModelResponse::assistant(
                "written and answered".to_string(),
            ),
            tinyagents_harness::tinyinference_llm::model::ModelResponse::assistant(
                "must not be reached".to_string(),
            ),
        ])),
    );
    harness.register_tool(Arc::new(FakeTool::returning("writer", "ok")));
    harness.with_policy(RunPolicy {
        limits: RunLimits::default()
            .with_max_model_calls(20)
            .with_max_tool_calls(20),
        ..RunPolicy::default()
    });
    harness.push_middleware(Arc::new(UnmetDeliverableMiddleware::new(None)));

    let run = harness
        .invoke_default(&(), vec![Message::user(request)])
        .await
        .expect("run succeeds");

    assert_eq!(run.text().as_deref(), Some("written and answered"));
    let notices = run
        .messages
        .iter()
        .filter(|m| matches!(m, Message::User(_)) && m.text().contains("does not exist"))
        .count();
    assert_eq!(
        notices, 1,
        "the notice must be given once, not every answer"
    );
}

/// A request that names nothing, or names only files that exist, must cost the
/// turn nothing: the first answer stands.
#[tokio::test]
async fn a_request_naming_no_missing_file_is_left_alone() {
    let mut harness: AgentHarness<()> = AgentHarness::new();
    harness.register_model(
        "mock",
        Arc::new(ScriptedModel::new(vec![
            tool_round("c0", "writer"),
            tinyagents_harness::tinyinference_llm::model::ModelResponse::assistant(
                "done".to_string(),
            ),
            tinyagents_harness::tinyinference_llm::model::ModelResponse::assistant(
                "must not be reached".to_string(),
            ),
        ])),
    );
    harness.register_tool(Arc::new(FakeTool::returning("writer", "ok")));
    harness.with_policy(RunPolicy {
        limits: RunLimits::default().with_max_model_calls(20),
        ..RunPolicy::default()
    });
    harness.push_middleware(Arc::new(UnmetDeliverableMiddleware::new(None)));

    let run = harness
        .invoke_default(&(), vec![Message::user("summarise the situation for me")])
        .await
        .expect("run succeeds");
    assert_eq!(run.text().as_deref(), Some("done"));
}

/// `install` puts the check on a root orchestrator turn with the policy's wall
/// clock, and on nothing else: a sub-agent's answer goes to its parent.
#[tokio::test]
async fn install_covers_root_orchestrator_turns_only() {
    let missing =
        std::env::temp_dir().join(format!("oh-unmet-install-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&missing);
    let request = format!("Do the work and write it to {}", missing.display());
    let script = || {
        Arc::new(ScriptedModel::new(vec![
            tool_round("c0", "writer"),
            tinyagents_harness::tinyinference_llm::model::ModelResponse::assistant(
                "here is what I found".to_string(),
            ),
            tool_round("c1", "writer"),
            tinyagents_harness::tinyinference_llm::model::ModelResponse::assistant(
                "written and answered".to_string(),
            ),
        ]))
    };
    let policy = || RunPolicy {
        limits: RunLimits::default()
            .with_max_model_calls(20)
            .with_max_tool_calls(20)
            .with_max_wall_clock_ms(Some(600_000)),
        ..RunPolicy::default()
    };

    let mut root: AgentHarness<()> = AgentHarness::new();
    root.register_model("mock", script());
    root.register_tool(Arc::new(FakeTool::returning("writer", "ok")));
    root.with_policy(policy());
    install(&mut root, false, Some("orchestrator"));
    let run = root
        .invoke_default(&(), vec![Message::user(request.clone())])
        .await
        .expect("run succeeds");
    assert_eq!(
        run.text().as_deref(),
        Some("written and answered"),
        "a root orchestrator turn is held for the file it never wrote"
    );

    let mut sub: AgentHarness<()> = AgentHarness::new();
    sub.register_model("mock", script());
    sub.register_tool(Arc::new(FakeTool::returning("writer", "ok")));
    sub.with_policy(policy());
    install(&mut sub, true, Some("orchestrator"));
    let run = sub
        .invoke_default(&(), vec![Message::user(request)])
        .await
        .expect("run succeeds");
    assert_eq!(
        run.text().as_deref(),
        Some("here is what I found"),
        "a sub-agent turn is not checked"
    );
}

/// The candidates come from the current turn's request, not from an earlier
/// turn of the thread that rides ahead of it in the harness input, and not
/// from a wrapped harness instruction.
#[tokio::test]
async fn candidates_come_from_the_current_turns_request() {
    let old = std::env::temp_dir().join(format!("oh-unmet-old-{}.json", std::process::id()));
    let new = std::env::temp_dir().join(format!("oh-unmet-new-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&old);
    let _ = std::fs::remove_file(&new);
    let mut harness: AgentHarness<()> = AgentHarness::new();
    harness.register_model(
        "mock",
        Arc::new(ScriptedModel::new(vec![
            tool_round("c0", "writer"),
            tinyagents_harness::tinyinference_llm::model::ModelResponse::assistant(
                "first answer".to_string(),
            ),
            tinyagents_harness::tinyinference_llm::model::ModelResponse::assistant(
                "second answer".to_string(),
            ),
        ])),
    );
    harness.register_tool(Arc::new(FakeTool::returning("writer", "ok")));
    harness.with_policy(RunPolicy {
        limits: RunLimits::default()
            .with_max_model_calls(20)
            .with_max_tool_calls(20),
        ..RunPolicy::default()
    });
    harness.push_middleware(Arc::new(UnmetDeliverableMiddleware::new(None)));
    let run = harness
        .invoke_default(
            &(),
            vec![
                Message::user(format!("earlier turn: write {}", old.display())),
                Message::assistant("done earlier"),
                Message::user(format!("now write {}", new.display())),
            ],
        )
        .await
        .expect("run succeeds");
    let notice = run
        .messages
        .iter()
        .filter(|m| matches!(m, Message::User(_)) && m.text().contains("does not exist"))
        .map(|m| m.text())
        .collect::<Vec<_>>();
    assert_eq!(notice.len(), 1, "held once for the current request");
    assert!(
        notice[0].contains(&new.display().to_string()),
        "names the current turn's path"
    );
    assert!(
        !notice[0].contains(&old.display().to_string()),
        "not the earlier turn's path"
    );
    assert_eq!(run.text().as_deref(), Some("second answer"));
}

fn result_text(result: &TaToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|block| match block {
            ToolContent::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// The two clock-driven notes ride a tool result once each: the half-time
/// note names the path still absent at 50% of the budget, the late note at
/// 80% says to stop exploring, and neither repeats in a later band. A
/// directory at the path counts as absent: the deliverable is a file.
#[tokio::test]
async fn the_half_time_and_late_notes_ride_a_tool_result_once_each() {
    let missing = std::env::temp_dir().join(format!("oh-unmet-clock-{}.csv", std::process::id()));
    let _ = std::fs::remove_dir_all(&missing);
    std::fs::create_dir_all(&missing).expect("a directory at the csv path");
    // A 4 s budget: the sleeps land a full band away from each threshold,
    // so a second of scheduler slop cannot move a call across one.
    let middleware = UnmetDeliverableMiddleware::new(Some(std::time::Duration::from_millis(4_000)));
    let mut ctx = RunContext::new(RunConfig::new("clock"), ());
    let mut request = ModelRequest {
        messages: vec![Message::user(format!(
            "do the work and write {}",
            missing.display()
        ))],
        ..ModelRequest::default()
    };
    Middleware::<(), ()>::before_model(&middleware, &mut ctx, &(), &mut request)
        .await
        .expect("before_model");
    async fn run(middleware: &UnmetDeliverableMiddleware, ctx: &mut RunContext<()>) -> String {
        let identity = tinyagents_harness::middleware::ToolInvocationIdentity::new("c0", "shell");
        let mut result = TaToolResult::success("output");
        Middleware::<(), ()>::after_tool(middleware, ctx, &(), &identity, &mut result)
            .await
            .expect("after_tool");
        result_text(&result)
    }

    assert_eq!(
        run(&middleware, &mut ctx).await,
        "output",
        "nothing before half-time"
    );

    std::thread::sleep(std::time::Duration::from_millis(2_200));
    let half = run(&middleware, &mut ctx).await;
    assert!(half.contains("Half the turn's budget is gone"), "{half:?}");
    assert!(
        half.contains(&missing.display().to_string()),
        "names the absent path"
    );
    assert_eq!(
        run(&middleware, &mut ctx).await,
        "output",
        "the half-time note is given once"
    );

    std::thread::sleep(std::time::Duration::from_millis(1_300));
    let late = run(&middleware, &mut ctx).await;
    assert!(late.contains("Stop exploring"), "{late:?}");
    assert!(
        !late.contains("Half the turn's budget"),
        "the late note stands alone"
    );
    assert_eq!(
        run(&middleware, &mut ctx).await,
        "output",
        "the late note is given once"
    );
    let _ = std::fs::remove_dir_all(&missing);
}

/// A directory at a path that names a file is not the deliverable.
#[test]
fn a_directory_at_the_path_is_still_missing() {
    let dir = std::env::temp_dir().join(format!("oh-unmet-dir-{}.json", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    let candidates = vec![dir.to_string_lossy().to_string()];
    assert_eq!(UnmetDeliverableMiddleware::missing(&candidates), candidates);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A run whose first tool result arrives past 80% gets the late note and
/// never the half-time one: the later rung supersedes the earlier.
#[tokio::test]
async fn a_late_first_observation_skips_the_half_time_note() {
    let missing = std::env::temp_dir().join(format!("oh-unmet-late-{}.csv", std::process::id()));
    let _ = std::fs::remove_file(&missing);
    let middleware = UnmetDeliverableMiddleware::new(Some(std::time::Duration::from_millis(1)));
    let mut ctx = RunContext::new(RunConfig::new("late-first"), ());
    let mut request = ModelRequest {
        messages: vec![Message::user(format!("write {}", missing.display()))],
        ..ModelRequest::default()
    };
    Middleware::<(), ()>::before_model(&middleware, &mut ctx, &(), &mut request)
        .await
        .expect("before_model");
    std::thread::sleep(std::time::Duration::from_millis(5));
    let identity = tinyagents_harness::middleware::ToolInvocationIdentity::new("c0", "shell");
    let mut first = TaToolResult::success("output");
    Middleware::<(), ()>::after_tool(&middleware, &mut ctx, &(), &identity, &mut first)
        .await
        .expect("after_tool");
    assert!(result_text(&first).contains("Stop exploring"));
    let mut second = TaToolResult::success("output");
    Middleware::<(), ()>::after_tool(&middleware, &mut ctx, &(), &identity, &mut second)
        .await
        .expect("after_tool");
    assert_eq!(
        result_text(&second),
        "output",
        "no half-time note after the late one"
    );
}

/// The turn's state is dropped when the run ends, so a long-lived harness
/// keeps nothing for runs it will not see again.
#[tokio::test]
async fn the_runs_state_is_dropped_when_the_turn_ends() {
    let missing = std::env::temp_dir().join(format!("oh-unmet-drop-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&missing);
    let middleware = Arc::new(UnmetDeliverableMiddleware::new(None));
    let mut harness: AgentHarness<()> = AgentHarness::new();
    harness.register_model(
        "mock",
        Arc::new(ScriptedModel::new(vec![
            tool_round("c0", "writer"),
            tinyagents_harness::tinyinference_llm::model::ModelResponse::assistant(
                "first answer".to_string(),
            ),
            tinyagents_harness::tinyinference_llm::model::ModelResponse::assistant(
                "second answer".to_string(),
            ),
        ])),
    );
    harness.register_tool(Arc::new(FakeTool::returning("writer", "ok")));
    harness.with_policy(RunPolicy {
        limits: RunLimits::default()
            .with_max_model_calls(20)
            .with_max_tool_calls(20),
        ..RunPolicy::default()
    });
    harness.push_middleware(Arc::clone(&middleware) as Arc<dyn Middleware<(), ()>>);
    let run = harness
        .invoke_default(
            &(),
            vec![Message::user(format!("write {}", missing.display()))],
        )
        .await
        .expect("run succeeds");
    assert_eq!(run.text().as_deref(), Some("second answer"));
    assert!(
        middleware.runs.lock().expect("lock").is_empty(),
        "the finished run leaves no state behind"
    );
}
