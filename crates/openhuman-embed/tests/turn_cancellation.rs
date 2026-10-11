//! Awaited cancellation stops one turn while leaving its agent reusable.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default)]
struct Outcomes(Mutex<Vec<openhuman_embed::observe::TurnFailure>>);
impl openhuman_embed::observe::TurnObserver for Outcomes {
    fn on_turn(&self, trace: &openhuman_embed::observe::TurnTrace<'_>) {
        assert!(trace.message.is_none());
        assert!(trace.reply.is_none());
        self.0.lock().unwrap().push(trace.failure.unwrap());
    }
}

use common::{offline_config, provider, route, runtime, stub_backend};
use openhuman_embed::{
    Access, AgentDefinitionSpec, AgentSpec, CoreError, Runtime, ToolScopeSpec, Workspace,
};

#[test]
fn cancellation_is_awaited_and_scoped_to_one_turn() {
    runtime().block_on(async {
        tokio::spawn(scenario()).await.unwrap();
    });
}

async fn scenario() {
    let backend = stub_backend().await;
    let provider = provider("finished").await;
    let runtime = Runtime::builder()
        .config(offline_config())
        .workspace(Workspace::Ephemeral)
        .backend_url(backend.uri())
        .build()
        .await
        .unwrap();
    let agent = runtime
        .agent(AgentSpec::new("worker").provider(route(&provider, "test-model")))
        .unwrap();

    // Cancellation before send never reaches inference and cannot hang.
    let metered = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = metered.clone();
    let mut turn = agent.turn("cancel before sending").meter(move |usage| {
        assert!(usage.is_none());
        observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    });
    let cancel = turn.cancellation_handle();
    tokio::time::timeout(Duration::from_secs(2), cancel.cancel())
        .await
        .unwrap();
    assert!(matches!(
        turn.send().await,
        Err(CoreError::TurnCancelled { .. })
    ));
    assert!(common::chat_requests(&provider).await.is_empty());
    assert_eq!(
        metered.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "cancelled turn is metered once"
    );

    // A later turn on the same agent remains usable.
    let mut turn = agent.turn("answer normally");
    let cancel = turn.cancellation_handle();
    assert_eq!(turn.send().await.unwrap().reply, "finished");
    tokio::time::timeout(Duration::from_secs(2), cancel.cancel())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), cancel.cancel())
        .await
        .unwrap();
    // Stop an active model request while another agent remains usable.
    let slow = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(common::chat_completion("too late"))
                .set_delay(Duration::from_secs(30)),
        )
        .mount(&slow)
        .await;
    let blocked = runtime
        .agent(AgentSpec::new("blocked").provider(route(&slow, "test-model")))
        .unwrap();
    let observed = metered.clone();
    let mut turn = blocked.turn("wait for inference").meter(move |_| {
        observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    });
    let cancel = turn.cancellation_handle();
    let sent = tokio::spawn(turn.send());
    tokio::time::timeout(Duration::from_secs(5), async {
        while common::chat_requests(&slow).await.is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let clone = cancel.clone();
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(cancel.cancel(), clone.cancel());
    })
    .await
    .unwrap();
    assert!(matches!(
        sent.await.unwrap(),
        Err(CoreError::TurnCancelled { .. })
    ));
    assert_eq!(metered.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(agent.run("still usable").await.unwrap().reply, "finished");

    let observed = Arc::new(Outcomes::default());
    // A whole-turn deadline drops inference and leaves the same agent reusable.
    let deadline = blocked
        .turn("deadline")
        .timeout(Duration::from_millis(100))
        .observer(observed.clone())
        .send()
        .await;
    assert!(
        matches!(deadline, Err(CoreError::DeadlineExceeded { .. })),
        "{deadline:?}"
    );
    assert_eq!(agent.run("after deadline").await.unwrap().reply, "finished");
    assert_eq!(
        *observed.0.lock().unwrap(),
        vec![openhuman_embed::observe::TurnFailure::Deadline]
    );

    // An externally dropped send future also acknowledges cancellation.
    let prior_requests = common::chat_requests(&slow).await.len();
    let observed = metered.clone();
    let mut turn = blocked.turn("drop this inference request").meter(move |_| {
        observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    });

    let cancel = turn.cancellation_handle();
    let sent = tokio::spawn(turn.send());
    tokio::time::timeout(Duration::from_secs(5), async {
        while common::chat_requests(&slow).await.len() <= prior_requests {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    sent.abort();
    assert!(sent.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(2), cancel.cancel())
        .await
        .unwrap();

    assert_eq!(metered.load(std::sync::atomic::Ordering::SeqCst), 3);

    let mut invalid = agent
        .turn("invalid route")
        .route(openhuman_embed::Route::openai_compatible(
            "https://example.invalid/v1",
            "",
        ));
    let cancel = invalid.cancellation_handle();
    assert!(matches!(
        invalid.send().await,
        Err(CoreError::InvalidRoute { .. })
    ));
    tokio::time::timeout(Duration::from_secs(2), cancel.cancel())
        .await
        .unwrap();

    #[cfg(target_os = "linux")]
    {
        // Exercise the builtin shell with no tool timeout, including a
        // grandchild. A cancel acknowledgement must follow reaping.
        let scratch = tempfile::tempdir().unwrap();
        let pidfile = scratch.path().join("child.pid");
        let shell_provider = common::scripted_provider(vec![common::tool_call_completion(
                "shell", &serde_json::json!({"command": format!("sleep 30 & echo $! > {}; wait", pidfile.display())}).to_string()
            )], "should not finish").await;
        let shell = runtime
            .agent(
                AgentSpec::new("shell-worker")
                    .provider(route(&shell_provider, "test-model"))
                    .access(Access::full())
                    .action_dir(scratch.path())
                    .definition(
                        AgentDefinitionSpec::new()
                            .tools(ToolScopeSpec::Named(vec!["shell".into()])),
                    ),
            )
            .unwrap();
        let mut turn = shell.turn("Run the shell command.");
        let cancel = turn.cancellation_handle();
        let sent = tokio::spawn(turn.send());
        let child: i32 = common::eventually("running shell child", || {
            std::fs::read_to_string(&pidfile).ok()?.trim().parse().ok()
        })
        .await;
        let cancelled = tokio::time::timeout(Duration::from_secs(5), cancel.cancel()).await;
        let live = std::fs::read_to_string(format!("/proc/{child}/stat")).is_ok_and(|s| {
            !s.rsplit_once(") ")
                .is_some_and(|(_, rest)| rest.starts_with('Z'))
        });
        // Always clean up if the regression returns.
        if live {
            let _ = std::process::Command::new("kill")
                .args(["-KILL", &child.to_string()])
                .status();
        }
        cancelled.unwrap();
        assert!(!live, "shell child survived awaited cancellation");
        assert!(matches!(
            sent.await.unwrap(),
            Err(CoreError::TurnCancelled { .. })
        ));
        assert_eq!(
            shell.run("answer normally").await.unwrap().reply,
            "should not finish"
        );
    }
}
