//! Lifecycle and protocol-error tests for the Phase 1 harness-script runner.
//!
//! Each test needs a real Python interpreter. To avoid any network download,
//! the harness prefers a system Python and **skips cleanly** when none that
//! meets the minimum version is present — mirroring
//! `runtime_python/bootstrap_tests.rs`. Failure paths that a real, well-behaved
//! child would never hit (bad handshake, floods) are driven through the
//! `with_bootstrap_source` test seam.

use super::runner::{HarnessScriptError, PythonHarnessScriptRunner, ScriptRunSpec};
use crate::openhuman::config::schema::RuntimePythonConfig;
use crate::openhuman::runtime_python::{detect_system_python, PythonBootstrap};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Build a bootstrap over a system Python, or `None` if the host has no
/// suitable interpreter (in which case the caller should skip).
fn system_bootstrap() -> Option<Arc<PythonBootstrap>> {
    let mut cfg = RuntimePythonConfig::default();
    cfg.prefer_system = true;
    cfg.minimum_version = "3.8".to_string();
    // Confirm a system interpreter exists up front so `resolve()` never falls
    // through to a managed download during tests.
    detect_system_python(&cfg.minimum_version, None)?;
    Some(Arc::new(PythonBootstrap::new(cfg)))
}

/// Skip-with-message helper so a missing interpreter is visible, not silent.
macro_rules! bootstrap_or_skip {
    () => {
        match system_bootstrap() {
            Some(b) => b,
            None => {
                eprintln!("skipping: no system python >= 3.8 available");
                return;
            }
        }
    };
}

#[tokio::test]
async fn happy_path_round_trips_inputs() {
    let bootstrap = bootstrap_or_skip!();
    let runner = PythonHarnessScriptRunner::new(bootstrap);

    let inputs = json!({"a": 1, "nested": ["x", "y"]});
    let spec = ScriptRunSpec::new("set_result(inputs)", inputs.clone());

    let outcome = runner
        .run(spec, CancellationToken::new())
        .await
        .expect("script should succeed");

    assert_eq!(outcome.output, inputs);
    assert!(!outcome.stderr_truncated);
}

#[tokio::test]
async fn result_defaults_to_null_when_unset() {
    let bootstrap = bootstrap_or_skip!();
    let runner = PythonHarnessScriptRunner::new(bootstrap);

    let spec = ScriptRunSpec::new("x = 1 + 1", json!(null));
    let outcome = runner
        .run(spec, CancellationToken::new())
        .await
        .expect("script should succeed");

    assert_eq!(outcome.output, json!(null));
}

#[tokio::test]
async fn script_stdout_does_not_corrupt_the_protocol() {
    let bootstrap = bootstrap_or_skip!();
    let runner = PythonHarnessScriptRunner::new(bootstrap);

    // A script that prints to stdout before setting a result must still yield a
    // clean `result`; the print goes to stderr, not the protocol stream.
    let spec = ScriptRunSpec::new("print('noisy diagnostic')\nset_result(inputs)", json!(42));
    let outcome = runner
        .run(spec, CancellationToken::new())
        .await
        .expect("script should succeed despite printing");

    assert_eq!(outcome.output, json!(42));
}

#[tokio::test]
async fn non_serializable_result_is_reported_as_script_error() {
    let bootstrap = bootstrap_or_skip!();
    let runner = PythonHarnessScriptRunner::new(bootstrap);

    // A value json can't encode must surface as a ScriptError, not as a missing
    // terminal frame.
    let spec = ScriptRunSpec::new("set_result(set([1, 2, 3]))", json!(null));
    let err = runner
        .run(spec, CancellationToken::new())
        .await
        .expect_err("non-serializable result should fail");

    match err {
        HarnessScriptError::ScriptError { error_class, .. } => {
            assert_eq!(error_class, "TypeError");
        }
        other => panic!("expected ScriptError, got {other:?}"),
    }
}

#[tokio::test]
async fn script_exception_is_reported_as_script_error() {
    let bootstrap = bootstrap_or_skip!();
    let runner = PythonHarnessScriptRunner::new(bootstrap);

    let spec = ScriptRunSpec::new("raise ValueError('boom')", json!(null));
    let err = runner
        .run(spec, CancellationToken::new())
        .await
        .expect_err("script should fail");

    match err {
        HarnessScriptError::ScriptError {
            error_class,
            message,
            traceback,
        } => {
            assert_eq!(error_class, "ValueError");
            assert!(message.contains("boom"), "message was {message:?}");
            assert!(traceback.is_some());
        }
        other => panic!("expected ScriptError, got {other:?}"),
    }
}

#[tokio::test]
async fn wall_timeout_kills_a_sleeping_child() {
    let bootstrap = bootstrap_or_skip!();
    let runner = PythonHarnessScriptRunner::new(bootstrap);

    let mut spec = ScriptRunSpec::new("import time\ntime.sleep(30)", json!(null));
    spec.wall_time = Duration::from_millis(500);

    let err = runner
        .run(spec, CancellationToken::new())
        .await
        .expect_err("run should time out");

    assert!(
        matches!(err, HarnessScriptError::WallTimeout(_)),
        "expected WallTimeout, got {err:?}"
    );
}

#[tokio::test]
async fn cancellation_kills_a_running_child() {
    let bootstrap = bootstrap_or_skip!();
    let runner = PythonHarnessScriptRunner::new(bootstrap);

    let mut spec = ScriptRunSpec::new("import time\ntime.sleep(30)", json!(null));
    spec.wall_time = Duration::from_secs(30);

    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        trigger.cancel();
    });

    let err = runner
        .run(spec, cancel)
        .await
        .expect_err("run should be cancelled");

    assert!(
        matches!(err, HarnessScriptError::Cancelled),
        "expected Cancelled, got {err:?}"
    );
}

#[tokio::test]
async fn handshake_version_mismatch_is_detected() {
    let bootstrap = bootstrap_or_skip!();
    // Inject a child that announces a future protocol version.
    let bad = r#"
import sys, time
sys.stdout.write('{"type":"ready","protocol_version":999}\n')
sys.stdout.flush()
time.sleep(5)
"#;
    let runner = PythonHarnessScriptRunner::new(bootstrap).with_bootstrap_source(bad);

    let spec = ScriptRunSpec::new("set_result(1)", json!(null));
    let err = runner
        .run(spec, CancellationToken::new())
        .await
        .expect_err("handshake should be rejected");

    match err {
        HarnessScriptError::ProtocolMismatch { expected, actual } => {
            assert_eq!(expected, super::protocol::PROTOCOL_VERSION);
            assert_eq!(actual, 999);
        }
        other => panic!("expected ProtocolMismatch, got {other:?}"),
    }
}

#[tokio::test]
async fn garbage_handshake_is_malformed() {
    let bootstrap = bootstrap_or_skip!();
    let bad = r#"
import sys, time
sys.stdout.write('this is not json\n')
sys.stdout.flush()
time.sleep(5)
"#;
    let runner = PythonHarnessScriptRunner::new(bootstrap).with_bootstrap_source(bad);

    let spec = ScriptRunSpec::new("set_result(1)", json!(null));
    let err = runner
        .run(spec, CancellationToken::new())
        .await
        .expect_err("garbage handshake should fail");

    assert!(
        matches!(err, HarnessScriptError::MalformedMessage(_)),
        "expected MalformedMessage, got {err:?}"
    );
}

#[tokio::test]
async fn oversized_stdout_trips_the_output_cap() {
    let bootstrap = bootstrap_or_skip!();
    // Handshake correctly, read init, then emit one oversized line.
    let flood = r#"
import sys, json, time
sys.stdout.write(json.dumps({"type": "ready", "protocol_version": 1}) + "\n")
sys.stdout.flush()
sys.stdin.readline()
sys.stdout.write(json.dumps({"type": "noise", "data": "x" * 10000}) + "\n")
sys.stdout.flush()
time.sleep(5)
"#;
    let runner = PythonHarnessScriptRunner::new(bootstrap).with_bootstrap_source(flood);

    let mut spec = ScriptRunSpec::new("set_result(1)", json!(null));
    spec.max_stdout_bytes = 64; // first noise line is far larger

    let err = runner
        .run(spec, CancellationToken::new())
        .await
        .expect_err("flood should trip the cap");

    assert!(
        matches!(err, HarnessScriptError::OutputCapExceeded(64)),
        "expected OutputCapExceeded, got {err:?}"
    );
}
