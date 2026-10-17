//! Live integration test — makes a real x402 payment to twit.sh on Base.
//!
//! Run manually (requires a funded wallet):
//!   GGML_NATIVE=OFF cargo test -p openhuman-cli --features web3 --test x402_twit_sh_live -- --ignored --nocapture

use std::sync::Arc;

use openhuman_core::security::{AutonomyLevel, SecurityPolicy};
use openhuman_core::web3::x402;
use serde_json::json;
use tinytools::Tool;

#[tokio::test]
#[ignore = "live: makes a real x402 payment to twit.sh on Base; needs a funded wallet and network. Run: GGML_NATIVE=OFF cargo test -p openhuman-cli --features web3 --test x402_twit_sh_live -- --ignored --nocapture"]
async fn x402_pay_twit_sh_for_hal_finney_tweet() {
    env_logger::init();

    let tmp = tempfile::tempdir().unwrap();
    x402::init_ledger(tmp.path(), "test-session");

    // This ignored test explicitly opts into a paid request to one endpoint;
    // use the current host authorization seam rather than bypassing it.
    let security = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Full,
        workspace_dir: tmp.path().to_path_buf(),
        action_dir: tmp.path().to_path_buf(),
        ..SecurityPolicy::default()
    });
    let tool = openhuman_core::tools::http_request_tool(
        &openhuman_core::core::runtime::DispatchCtx::process_default(),
        security,
        vec!["x402.twit.sh".into()],
        0,
        0,
    );
    let result = tool
        .execute(json!({
            "url": "https://x402.twit.sh/tweets/by/id?id=1110302988",
            "method": "GET"
        }))
        .await
        .expect("tool execute should not panic");

    println!("=== x402 tool result ===");
    for content in &result.content {
        if let tinytools::ToolContent::Text { text } = content {
            println!("{text}");
        }
    }
    println!("is_error: {}", result.is_error);

    assert!(!result.is_error, "x402 request should succeed");
}
