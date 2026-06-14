//! Live integration test — makes a real x402 payment to twit.sh on Base.
//!
//! Run manually (requires a funded wallet):
//!   GGML_NATIVE=OFF cargo test --test x402_twit_sh_live -- --ignored --nocapture

use openhuman_core::openhuman::tools::traits::Tool;
use openhuman_core::openhuman::x402;
use serde_json::json;

#[tokio::test]
#[ignore] // requires funded wallet + network access
async fn x402_pay_twit_sh_for_hal_finney_tweet() {
    env_logger::init();

    let tmp = tempfile::tempdir().unwrap();
    x402::init_ledger(tmp.path(), "test-session");

    let tool = x402::tools::X402RequestTool::new();
    let result = tool
        .execute(json!({
            "url": "https://x402.twit.sh/tweets/by/id?id=1110302988",
            "method": "GET"
        }))
        .await
        .expect("tool execute should not panic");

    println!("=== x402 tool result ===");
    for content in &result.content {
        if let openhuman_core::openhuman::workflows::types::ToolContent::Text { text } = content {
            println!("{text}");
        }
    }
    println!("is_error: {}", result.is_error);

    assert!(!result.is_error, "x402 request should succeed");
}
