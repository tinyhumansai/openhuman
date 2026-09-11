//! Round 15 raw integration coverage for network tools plus web-channel paths.
//!
//! Everything here stays local-only: loopback HTTP mocks, temp git/cron
//! workspaces, and validation/error branches that do not touch the desktop.

use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::Router;
use serde_json::json;
use tempfile::{tempdir, TempDir};
use tokio::time::timeout;

use openhuman_core::core::socketio::WebChannelEvent;
use openhuman_core::openhuman::web_chat::{
    all_web_channel_controller_schemas, all_web_channel_registered_controllers, cancel_chat,
    channel_web_cancel, publish_web_channel_event, schemas as web_channel_schema, start_chat,
    subscribe_web_channel_events, ChatRequestMetadata,
};
use openhuman_core::openhuman::config::{AutonomyConfig, Config};
use openhuman_core::openhuman::security::{AutonomyLevel, SecurityPolicy};
use openhuman_core::openhuman::tools::{
    ComposioTool, GitOperationsTool, ScheduleTool, Tool, ToolCallOptions,
};

#[derive(Clone, Debug)]
struct MockRequest {
    method: Method,
    path: String,
    query: Option<String>,
    body: String,
}

#[derive(Clone, Default)]
struct MockState {
    requests: Arc<Mutex<Vec<MockRequest>>>,
}

fn full_security(workspace: &std::path::Path) -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy::from_config(
        &AutonomyConfig {
            level: AutonomyLevel::Full,
            max_actions_per_hour: 10_000,
            ..Default::default()
        },
        workspace,
        workspace,
    ))
}

fn readonly_security(workspace: &std::path::Path) -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy::from_config(
        &AutonomyConfig {
            level: AutonomyLevel::ReadOnly,
            max_actions_per_hour: 10_000,
            ..Default::default()
        },
        workspace,
        workspace,
    ))
}

fn temp_config() -> (TempDir, Config) {
    let tmp = tempdir().expect("tempdir");
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    config.config_path = tmp.path().join("config.toml");
    std::fs::create_dir_all(&config.workspace_dir).expect("workspace");
    (tmp, config)
}

fn text(result: &openhuman_core::openhuman::tools::ToolResult) -> String {
    result.output()
}

fn assert_contains(haystack: &str, needle: &str) {
    assert!(
        haystack.contains(needle),
        "expected {haystack:?} to contain {needle:?}"
    );
}

#[tokio::test]
async fn git_operations_cover_read_write_markdown_and_safety_rejections() {
    let tmp = tempdir().expect("repo tempdir");
    let repo = tmp.path();
    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "round15@example.test"]);
    run_git(repo, &["config", "user.name", "Round Fifteen"]);
    std::fs::write(repo.join("tracked.txt"), "first\n").expect("write tracked");
    run_git(repo, &["add", "tracked.txt"]);
    run_git(repo, &["commit", "-m", "initial"]);
    std::fs::write(repo.join("tracked.txt"), "first\nsecond\n").expect("modify tracked");
    std::fs::write(repo.join("untracked.txt"), "new\n").expect("write untracked");

    let tool = GitOperationsTool::new(full_security(repo), repo.to_path_buf());
    let status = tool
        .execute(json!({"operation": "status"}))
        .await
        .expect("status");
    assert!(!status.is_error);
    assert_contains(&text(&status), "untracked.txt");
    assert_contains(
        status.markdown_formatted.as_deref().unwrap_or(""),
        "untracked",
    );

    let diff = tool
        .execute(json!({"operation": "diff", "files": "tracked.txt"}))
        .await
        .expect("diff");
    assert!(!diff.is_error);
    assert_contains(&text(&diff), "second");

    let blocked_diff = tool
        .execute(json!({"operation": "diff", "files": "tracked.txt;rm"}))
        .await
        .expect_err("blocked diff should hard fail in sanitizer");
    assert_contains(&blocked_diff.to_string(), "Blocked potentially");

    let add = tool
        .execute(json!({"operation": "add", "paths": "tracked.txt"}))
        .await
        .expect("add");
    assert!(!add.is_error, "add failed: {}", text(&add));

    let commit = tool
        .execute(json!({"operation": "commit", "message": "\n round15 commit \n"}))
        .await
        .expect("commit");
    assert!(!commit.is_error, "commit failed: {}", text(&commit));

    let log = tool
        .execute(json!({"operation": "log", "limit": 2}))
        .await
        .expect("log");
    assert!(!log.is_error);
    assert_contains(&text(&log), "round15 commit");

    let branch = tool
        .execute_with_options(
            json!({"operation": "branch"}),
            ToolCallOptions {
                prefer_markdown: true,
            },
        )
        .await
        .expect("branch");
    assert!(!branch.is_error);
    assert_contains(
        branch.markdown_formatted.as_deref().unwrap_or(""),
        "current",
    );

    let bad_checkout = tool
        .execute(json!({"operation": "checkout", "branch": "main~1"}))
        .await
        .expect_err("invalid branch should be a hard validation error");
    assert_contains(&bad_checkout.to_string(), "invalid characters");

    let readonly = GitOperationsTool::new(readonly_security(repo), repo.to_path_buf());
    let blocked = readonly
        .execute(json!({"operation": "add", "paths": "untracked.txt"}))
        .await
        .expect("readonly add");
    assert!(blocked.is_error);
    assert_contains(&text(&blocked), "[policy-blocked]");
}

#[tokio::test]
async fn schedule_tool_covers_cron_once_agent_prompt_and_policy_edges() {
    let (_tmp, config) = temp_config();
    let tool = ScheduleTool::new(full_security(&config.workspace_dir), config.clone());

    let empty = tool.execute(json!({"action": "list"})).await.expect("list");
    assert!(!empty.is_error);
    assert_contains(&text(&empty), "No scheduled jobs");

    let natural_language = tool
        .execute(json!({
            "action": "create",
            "delay": "30m",
            "command": "remind me to stretch",
            "name": "stretch"
        }))
        .await
        .expect("agent prompt");
    assert!(!natural_language.is_error, "{}", text(&natural_language));
    assert_contains(&text(&natural_language), "Created agent job");

    let recurring = tool
        .execute(json!({
            "action": "add",
            "expression": "*/15 * * * *",
            "command": "echo round15"
        }))
        .await
        .expect("recurring");
    assert!(!recurring.is_error, "{}", text(&recurring));
    let recurring_id = text(&recurring)
        .split_whitespace()
        .nth(3)
        .expect("job id")
        .to_string();

    let once = tool
        .execute(json!({
            "action": "once",
            "run_at": "2035-01-01T00:00:00Z",
            "command": "echo future"
        }))
        .await
        .expect("once");
    assert!(!once.is_error, "{}", text(&once));

    let list = tool.execute(json!({"action": "list"})).await.expect("list");
    assert!(!list.is_error);
    assert_contains(&text(&list), "echo round15");
    assert_contains(&text(&list), "[one-shot]");

    let get = tool
        .execute(json!({"action": "get", "id": recurring_id}))
        .await
        .expect("get");
    assert!(!get.is_error);
    assert_contains(&text(&get), "echo round15");

    let id = text(&get)
        .lines()
        .find(|line| line.contains("\"id\""))
        .and_then(|line| line.split('"').nth(3))
        .expect("json id")
        .to_string();

    for action in ["pause", "resume", "cancel"] {
        let result = tool
            .execute(json!({"action": action, "id": id}))
            .await
            .unwrap_or_else(|err| panic!("{action}: {err}"));
        assert!(!result.is_error, "{action} failed: {}", text(&result));
    }

    let missing_command = tool
        .execute(json!({"action": "create", "expression": "* * * * *"}))
        .await
        .expect("missing command");
    assert!(missing_command.is_error);
    assert_contains(&text(&missing_command), "Provide 'command'");

    let invalid_once = tool
        .execute(json!({
            "action": "once",
            "delay": "5m",
            "run_at": "2035-01-01T00:00:00Z",
            "command": "echo invalid"
        }))
        .await
        .expect("invalid once");
    assert!(invalid_once.is_error);
    assert_contains(&text(&invalid_once), "not both");

    let readonly = ScheduleTool::new(readonly_security(&config.workspace_dir), config);
    let blocked = readonly
        .execute(json!({
            "action": "create",
            "expression": "* * * * *",
            "command": "echo blocked"
        }))
        .await
        .expect("readonly create");
    assert!(blocked.is_error);
    assert_contains(&text(&blocked), "read-only");
}

#[tokio::test]
async fn composio_direct_and_mouse_tools_cover_validation_policy_and_schema_paths() {
    let (_tmp, config) = temp_config();
    let full = full_security(&config.workspace_dir);
    let readonly = readonly_security(&config.workspace_dir);

    let composio = ComposioTool::new("  local-test-key  ", Some(" entity-1 "), full.clone());
    assert_eq!(composio.name(), "composio");
    assert!(composio.external_effect());
    assert!(!composio.external_effect_with_args(&json!({"action": "list"})));
    assert!(!composio.external_effect_with_args(&json!({"action": "connect"})));
    assert!(composio.external_effect_with_args(&json!({"action": "execute"})));
    assert_contains(
        &composio.parameters_schema().to_string(),
        "connected_account_id",
    );

    let unknown = composio
        .execute(json!({"action": "wat"}))
        .await
        .expect("unknown composio");
    assert!(unknown.is_error);
    assert_contains(&text(&unknown), "Unknown action");

    let missing_connect = composio
        .execute(json!({"action": "connect"}))
        .await
        .expect_err("connect without app/auth_config_id should hard fail before network");
    assert_contains(&missing_connect.to_string(), "Missing 'app'");

    let readonly_composio = ComposioTool::new("local-test-key", None, readonly.clone());
    let blocked_execute = readonly_composio
        .execute(json!({
            "action": "execute",
            "tool_slug": "GMAIL_SEND_EMAIL",
            "params": { "to": "nobody@example.test" }
        }))
        .await
        .expect("readonly execute");
    assert!(blocked_execute.is_error);
    assert_contains(&text(&blocked_execute), "policy");
}

#[tokio::test]
async fn web_channel_public_paths_cover_validation_cancel_schema_and_event_bus() {
    assert_eq!(all_web_channel_controller_schemas().len(), 4);
    assert_eq!(all_web_channel_registered_controllers().len(), 4);
    assert_eq!(web_channel_schema("chat").function, "web_chat");
    assert_eq!(web_channel_schema("cancel").function, "web_cancel");
    assert_eq!(web_channel_schema("missing").function, "unknown");

    let missing_client = start_chat(
        " ",
        "thread",
        "hello",
        None,
        None,
        None,
        None,
        None,
        ChatRequestMetadata::default(),
    )
    .await
    .expect_err("blank client");
    assert_contains(&missing_client, "client_id is required");
    let missing_thread = cancel_chat("client", " ").await.expect_err("blank thread");
    assert_contains(&missing_thread, "thread_id is required");

    let none = cancel_chat("client", "round15-thread")
        .await
        .expect("no in-flight cancel");
    assert_eq!(none, None);

    let outcome = channel_web_cancel(" client ", " round15-thread ", None)
        .await
        .expect("cancel rpc outcome");
    assert_eq!(outcome.value["cancelled"], false);
    assert_eq!(outcome.value["client_id"], "client");
    assert_eq!(outcome.value["thread_id"], "round15-thread");

    let mut rx = subscribe_web_channel_events();
    publish_web_channel_event(WebChannelEvent {
        event: "round15_probe".to_string(),
        client_id: "client".to_string(),
        thread_id: "thread".to_string(),
        request_id: "request".to_string(),
        message: Some("payload".to_string()),
        ..Default::default()
    });
    let event = timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("event timeout")
        .expect("event");
    assert_eq!(event.event, "round15_probe");
    assert_eq!(event.message.as_deref(), Some("payload"));
}

/// Run `git` in `repo` with the developer's own git configuration closed out.
///
/// The fixture below performs a real `commit`, and without this it inherits
/// whatever the machine running it happens to configure. A global
/// `commit.gpgsign = true` — which every maintainer who signs commits has, and
/// which this repository's own contributing guide asks for — makes that commit
/// try to sign, and it fails with `error: gpg failed to sign the data` for
/// reasons that have nothing to do with the code under test. CI has no global
/// git config, so the test is green there and red only on the laptops of the
/// people most likely to be running it.
///
/// `GIT_CONFIG_GLOBAL` must name a readable-but-empty path rather than be
/// unset: unsetting it lets git fall back to `~/.gitconfig`, which is the thing
/// being closed. This mirrors `NULL_CONFIG_PATH` and `suppress_ambient_git_config`
/// in `tools/impl/filesystem/git_operations_config.rs`, and the unit suite's
/// own `hermetic()` helper.
///
/// The committer identity is unaffected: it is set repository-locally at the
/// top of the fixture, so closing the global config does not strand the commit
/// the way it would if the identity were ambient too.
fn run_git(repo: &std::path::Path, args: &[&str]) {
    // `/dev/null` is not a path on Windows; `NUL` is.
    let null_config = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", null_config)
        .output()
        .expect("spawn git");
    assert!(
        output.status.success(),
        "git {args:?} failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// #5494 / #5672 — a repository config that names a command does not get to run it,
/// asserted through the live agent tool surface rather than the private helper.
///
/// The tool's own workspace is agent-writable, and several git config keys name a
/// program git then executes — `core.fsmonitor` is run by `git status`, which every
/// operation this tool exposes reaches through `run_git_command_in`. So an agent that
/// can write a file can choose what the next `status` executes.
///
/// The unit suite in `git_operations_tests.rs` already pins the predicate. What it
/// cannot show is that the *tool surface an agent actually calls* is the one sitting
/// behind the guard: a new operation, or a refactor that reaches git another way,
/// would restore the exploit with every unit test still green. That is what this
/// drives — `GitOperationsTool::execute`, the same entry the agent tool loop uses.
///
/// The marker assertion is the security one. An error message alone would not prove
/// the hook did not run: the guard could refuse *after* spawning git. The hook is
/// proven to work before it is planted, so a missing marker afterwards means it was
/// refused, not that the hook was silently broken.
#[cfg(unix)]
#[tokio::test]
async fn git_tool_refuses_a_workspace_repo_config_that_names_a_command() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempdir().expect("repo tempdir");
    let repo = tmp.path();
    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "guard@example.test"]);
    run_git(repo, &["config", "user.name", "Guard"]);

    // A hook that records the fact it ran. Proven to run on its own first, so a
    // later absent marker is evidence of refusal rather than of a broken fixture.
    let hook = repo.join("hook.sh");
    let marker = repo.join("COMMAND_RAN");
    std::fs::write(
        &hook,
        format!("#!/bin/sh\ntouch {:?}\nexit 1\n", marker.to_string_lossy()),
    )
    .expect("write hook");
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).expect("chmod hook");
    std::process::Command::new(&hook)
        .status()
        .expect("hook runs");
    assert!(marker.exists(), "the planted hook does not run at all");
    std::fs::remove_file(&marker).expect("clear marker");

    let hook_path = hook.to_string_lossy().into_owned();
    run_git(repo, &["config", "core.fsmonitor", &hook_path]);

    let tool = GitOperationsTool::new(full_security(repo), repo.to_path_buf());
    let refused = tool.execute(json!({"operation": "status"})).await;
    let message = match &refused {
        Ok(result) => result.output(),
        Err(err) => err.to_string(),
    };

    // Checked BEFORE anything about the result, deliberately: whether the call
    // reported an error is secondary, and asserting that first would abort the
    // test before it reached the question that matters. Remove the guard and
    // `status` succeeds, so an is-error assertion fires first and reports a
    // missing error rather than an executed command.
    //
    // THE assertion. An error alone would not prove anything — the guard could
    // refuse after spawning git, or git could fail for an unrelated reason. The
    // marker is the only evidence that the command named by the workspace's own
    // config never ran.
    assert!(
        !marker.exists(),
        "git executed the command named by the workspace's own repository config \
         (`core.fsmonitor`); the tool returned: {message}"
    );

    // Secondary: the call must not report success either.
    if let Ok(result) = &refused {
        assert!(
            result.is_error,
            "status under a hostile repository config reported success: {message}"
        );
    }

    // The refusal must also be legible. This was briefly not true: the
    // repository probe used to run through the guarded `run_git_command_in`,
    // so a refused probe was flattened by `is_ok_and` into the generic "Not in
    // a git repository" — false, and un-actionable, for a directory that
    // plainly is one. `25ea41efe` fixed that by probing with `hardened_git`
    // and asking the guard before concluding anything. Asserting the key here
    // keeps the diagnostic from regressing back to the generic message.
    assert_contains(&message, "core.fsmonitor");
}

/// The other half of #5672, and the reason the allowlist is not simply "refuse any
/// local config": an ordinary repository still works.
///
/// `git init` plus an identity is what every real workspace looks like, so a guard
/// that refused it would make the tool useless and would be reverted rather than
/// fixed. Pinning it here means a future tightening of `ALLOWED_REPO_CONFIG` that
/// breaks ordinary repositories fails in the e2e lane rather than in the field.
#[tokio::test]
async fn git_tool_still_runs_under_an_ordinary_repository_config() {
    let tmp = tempdir().expect("repo tempdir");
    let repo = tmp.path();
    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "ordinary@example.test"]);
    run_git(repo, &["config", "user.name", "Ordinary"]);
    std::fs::write(repo.join("visible.txt"), "hello\n").expect("write file");

    let tool = GitOperationsTool::new(full_security(repo), repo.to_path_buf());
    let status = tool
        .execute(json!({"operation": "status"}))
        .await
        .expect("status on an ordinary repo must not error");

    assert!(!status.is_error, "ordinary repo refused: {}", text(&status));
    assert_contains(&text(&status), "visible.txt");
}
