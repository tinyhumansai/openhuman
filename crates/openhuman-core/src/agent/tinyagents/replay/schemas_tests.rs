use super::*;
use crate::core::runtime::DispatchCtx;

#[tokio::test]
async fn run_events_rejects_missing_run_id() {
    let err = handle_run_events(DispatchCtx::process_default(), Map::new())
        .await
        .unwrap_err();
    assert!(err.contains("invalid params"), "{err}");
}
