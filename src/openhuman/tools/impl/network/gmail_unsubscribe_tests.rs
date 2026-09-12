use super::*;
use crate::openhuman::tools::traits::ToolContent;

#[tokio::test]
async fn test_gmail_unsubscribe_valid() {
    let tool = GmailUnsubscribeTool;
    let result = tool
        .execute(serde_json::json!({
            "sender": "marketing@example.com",
            "unsubscribe_link": "https://example.com/unsub"
        }))
        .await
        .unwrap();

    assert!(!result.is_error);
    let mut has_json = false;
    for content in result.content {
        if let ToolContent::Json { data: value } = content {
            assert_eq!(value["status"].as_str().unwrap(), "pending_approval");
            assert_eq!(value["action"].as_str().unwrap(), "unsubscribe");
            assert_eq!(
                value["metadata"]["sender"].as_str().unwrap(),
                "marketing@example.com"
            );
            assert_eq!(
                value["metadata"]["unsubscribe_link"].as_str().unwrap(),
                "https://example.com/unsub"
            );
            has_json = true;
        }
    }
    assert!(has_json, "Expected JSON result");
}

#[tokio::test]
async fn test_gmail_unsubscribe_empty_link() {
    let tool = GmailUnsubscribeTool;
    let result = tool
        .execute(serde_json::json!({
            "sender": "marketing@example.com",
            "unsubscribe_link": ""
        }))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result
        .text()
        .contains("without a valid List-Unsubscribe link"));
}

#[tokio::test]
async fn test_gmail_unsubscribe_missing_link() {
    let tool = GmailUnsubscribeTool;
    let result = tool
        .execute(serde_json::json!({
            "sender": "marketing@example.com"
        }))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result
        .text()
        .contains("without a valid List-Unsubscribe link"));
}
