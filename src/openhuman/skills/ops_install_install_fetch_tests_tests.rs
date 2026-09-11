use super::*;
use axum::{http::StatusCode, Router};
use tempfile::TempDir;

/// Spawn a throwaway local HTTP server whose every route answers with
/// `status`. Returns its `http://127.0.0.1:<port>` base.
async fn spawn_status(status: StatusCode) -> String {
    let app = Router::new().fallback(move || async move { status });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://127.0.0.1:{}", addr.port())
}

/// TAURI-RUST-CGE: a non-2xx skill-install fetch always returns the
/// user-facing `Err` (so the UI surfaces "skill not found" / the failure),
/// for both a 4xx (which is NOT reported to Sentry) and a 5xx (which is).
/// Exercises both branches of the non-2xx handling; the report-suppression
/// polarity itself is asserted by `is_skills_install_client_error_event` in
/// observability. Passes the local-HTTP install escape hatch as an
/// explicit param so the loopback mock passes URL validation — the env-var
/// path is process-global and races with other env-touching tests under
/// parallel execution (#4567).
#[tokio::test]
async fn non_2xx_install_fetch_returns_err_for_4xx_and_5xx() {
    let tmp = TempDir::new().unwrap();

    for status in [StatusCode::NOT_FOUND, StatusCode::INTERNAL_SERVER_ERROR] {
        let base = spawn_status(status).await;
        let url = format!("{base}/skill.md");
        let err = install_workflow_from_url_with_home(
            tmp.path(),
            InstallWorkflowFromUrlParams {
                url,
                timeout_secs: Some(5),
            },
            None,
            true,
        )
        .await
        .expect_err("a non-2xx fetch must return Err so the UI surfaces it");
        assert!(
            err.contains(&format!("returned status {}", status.as_u16())),
            "error must surface the status to the UI: {err}"
        );
    }
}
