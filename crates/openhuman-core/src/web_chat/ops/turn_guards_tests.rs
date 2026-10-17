use super::*;

fn result(text: &str) -> WebChatTaskResult {
    WebChatTaskResult {
        full_response: text.to_string(),
        citations: Vec::new(),
        usage: None,
        workspace_dir: std::path::PathBuf::from("/tmp/turn-guards"),
        timing: None,
    }
}

/// The guarded turn's host environment carries the backstop it runs under, so
/// the session turn can hand the harness the deadline it must wind down before.
#[test]
fn guarded_turn_env_carries_the_backstop_deadline() {
    let backstop = Duration::from_secs(900);
    let (env, _) = turn_host_env(Some(backstop));
    let deadline = env.turn_deadline.expect("deadline on the turn's env");
    assert_eq!(deadline.backstop(), backstop);
    assert_eq!(deadline.wind_down_after(), Duration::from_secs(780));
    assert!(deadline.wind_down_after() < deadline.hard_stop_after());
    assert!(deadline.hard_stop_after() < backstop);
}

#[test]
fn disabled_backstop_carries_no_deadline() {
    let (env, _) = turn_host_env(None);
    assert!(env.turn_deadline.is_none());
}

/// The env's cancellation is the very token a Stop fires.
#[test]
fn guarded_turn_env_shares_its_cancellation_with_the_stop_handle() {
    let (env, stop) = turn_host_env(None);
    let observed = env.cancellation.expect("cancellation on the turn's env");
    assert!(!observed.is_cancelled());
    stop.cancel();
    assert!(observed.is_cancelled());
}

/// If the backstop fires anyway, the caller gets the typed `turn_timeout`
/// error, which is classified to user-facing copy, not silence.
#[tokio::test(start_paused = true)]
async fn backstop_firing_yields_a_classified_turn_timeout_error() {
    let err = drive_turn_with_deadline(
        &crate::core::runtime::DispatchCtx::host_default(),
        Some(Duration::from_secs(900)),
        None,
        None,
        |_| async {
            std::future::pending::<()>().await;
            Ok(result("never"))
        },
    )
    .await
    .expect_err("backstop fires");
    assert!(super::super::super::web_errors::is_outer_backstop_timeout(
        &err
    ));
    let classified = super::super::super::web_errors::classify_inference_error(&err);
    assert_eq!(classified.error_type, "turn_timeout");
    assert!(!classified.message.trim().is_empty());
}

fn text_delta() -> crate::agent::progress::AgentProgress {
    crate::agent::progress::AgentProgress::TextDelta {
        delta: "x".into(),
        iteration: 1,
    }
}

const IDLE: Option<Duration> = Some(Duration::from_secs(900));
const CEILING: Option<Duration> = Some(Duration::from_secs(3_600));

/// A turn that keeps emitting progress runs past the old 900s backstop: the
/// idle guard restarts on every event and the backstop is now the 3600s
/// ceiling.
#[tokio::test(start_paused = true)]
async fn a_turn_that_keeps_progressing_outlives_the_idle_window() {
    let res = drive_turn_with_deadline(
        &crate::core::runtime::DispatchCtx::host_default(),
        CEILING,
        IDLE,
        None,
        |activity| async move {
            for _ in 0..90 {
                tokio::time::sleep(Duration::from_secs(17)).await;
                activity.observe(&text_delta());
            }
            Ok(result("done after 25 minutes"))
        },
    )
    .await
    .expect("idle guard must not fire on a progressing turn");
    assert_eq!(res.full_response, "done after 25 minutes");
}

/// A turn that goes silent with nothing in flight ends after the idle window,
/// long before the ceiling, with the classified `turn_timeout` error.
#[tokio::test(start_paused = true)]
async fn a_silent_turn_ends_after_the_idle_window() {
    let started = tokio::time::Instant::now();
    let err = drive_turn_with_deadline(
        &crate::core::runtime::DispatchCtx::host_default(),
        CEILING,
        IDLE,
        None,
        |_| async {
            std::future::pending::<()>().await;
            Ok(result("never"))
        },
    )
    .await
    .expect_err("idle guard fires");
    let waited = started.elapsed();
    assert!(waited >= Duration::from_secs(900) && waited < Duration::from_secs(3_600));
    assert!(err.contains("no progress for"), "{err}");
    assert!(super::super::super::web_errors::is_outer_backstop_timeout(
        &err
    ));
    let classified = super::super::super::web_errors::classify_inference_error(&err);
    assert_eq!(classified.error_type, "turn_timeout");
}

/// A turn that stays busy forever is still bounded by the fixed ceiling.
#[tokio::test(start_paused = true)]
async fn a_turn_busy_past_the_ceiling_hits_the_backstop() {
    let err = drive_turn_with_deadline(
        &crate::core::runtime::DispatchCtx::host_default(),
        CEILING,
        IDLE,
        None,
        |activity| async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                activity.observe(&text_delta());
            }
        },
    )
    .await
    .expect_err("backstop fires");
    assert!(err.contains("3600s wall-clock budget"), "{err}");
}

/// A Stop fires the turn's own session cancellation and lets the turn finish,
/// so the session runtime can persist the stopped turn's rounds (#7715). The
/// turn's result is still discarded: the caller unwinds as a cancel.
#[tokio::test]
async fn stop_fires_the_session_cancellation_and_lets_the_turn_unwind() {
    let cancel = CancellationToken::new();
    let unwound = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let unwound_in_turn = unwound.clone();
    let (env, session_cancellation) = turn_host_env(None);
    let observed = env.cancellation.expect("session cancellation");
    let turn = run_until_stopped(cancel.clone(), session_cancellation, async move {
        observed.cancelled().await;
        unwound_in_turn.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok::<_, String>(result("persisted partial"))
    });
    let stop = async {
        tokio::task::yield_now().await;
        cancel.cancel();
        // A double Stop is a no-op, not a second unwind.
        cancel.cancel();
    };
    let (res, ()) = tokio::join!(turn, stop);
    assert!(res.is_none(), "a stopped turn reports a cancel");
    assert!(
        unwound.load(std::sync::atomic::Ordering::SeqCst),
        "the turn observed its session cancellation and ran to completion"
    );
}

/// A turn that ignores its session cancellation is dropped once the grace
/// runs out, so a Stop can never hang on a stuck turn.
#[tokio::test(start_paused = true)]
async fn stop_drops_a_turn_that_ignores_cancellation_after_the_grace() {
    let cancel = CancellationToken::new();
    cancel.cancel();
    let started = tokio::time::Instant::now();
    let res = run_until_stopped(
        cancel,
        tinyagents_harness::cancel::CancellationToken::new(),
        std::future::pending::<()>(),
    )
    .await;
    assert!(res.is_none());
    assert!(started.elapsed() >= STOPPED_TURN_PERSIST_GRACE);
}

/// Without a Stop the turn's own result comes back.
#[tokio::test]
async fn unstopped_turn_returns_its_result() {
    let res = run_until_stopped(
        CancellationToken::new(),
        tinyagents_harness::cancel::CancellationToken::new(),
        async { Ok::<_, String>(result("answer")) },
    )
    .await;
    assert_eq!(res.expect("not stopped").unwrap().full_response, "answer");
}
