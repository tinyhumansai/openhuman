use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use serde_json::json;

use crate::mcp::ui::cache::{put_inline, InlineEntry};
use crate::mcp::ui::types::{UiCsp, UiToolDescriptor};
use crate::security::AutonomyLevel;

#[derive(Default)]
struct FakePort {
    descriptor: Option<UiToolDescriptor>,
    contents: Vec<Value>,
    calls: AtomicUsize,
    reads: AtomicUsize,
}

#[async_trait]
impl UiServerPort for FakePort {
    async fn is_available(&self, server_id: &str) -> bool {
        server_id == "srv"
    }
    async fn tool_descriptor(&self, _server_id: &str, tool: &str) -> Option<UiToolDescriptor> {
        (tool == "known").then(|| self.descriptor.clone().unwrap_or_default())
    }
    async fn read_resource(&self, _server_id: &str, _uri: &str) -> Result<Vec<Value>, String> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(self.contents.clone())
    }
    async fn call_tool(&self, _s: &str, _t: &str, arguments: Value) -> Result<Value, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"content": [], "structuredContent": arguments}))
    }
}

fn read_only() -> UiToolDescriptor {
    UiToolDescriptor {
        meta: None,
        annotations: Some(json!({"readOnlyHint": true})),
    }
}

#[tokio::test]
async fn resource_read_rejects_non_ui_and_unconnected() {
    let port = FakePort::default();
    let err = resource_read(&port, Some("srv".into()), Some("https://x/y".into()), None)
        .await
        .unwrap_err();
    assert!(err.contains("ui://"));
    let err = resource_read(&port, Some("other".into()), Some("ui://x".into()), None)
        .await
        .unwrap_err();
    assert!(err.contains("not connected"));
}

#[tokio::test]
async fn resource_read_rejects_oversize_and_caches() {
    let port = FakePort {
        contents: vec![
            json!({"uri": "ui://big", "mimeType": "text/html", "text": "x".repeat(crate::mcp::ui::resolve::MAX_RESOURCE_BYTES + 1)}),
        ],
        ..FakePort::default()
    };
    assert!(
        resource_read(&port, Some("srv".into()), Some("ui://big".into()), None)
            .await
            .is_err()
    );

    let port = FakePort {
        contents: vec![json!({"uri": "ui://ok-cache", "mimeType": "text/html", "text": "<p/>"})],
        ..FakePort::default()
    };
    for _ in 0..2 {
        let out = resource_read(
            &port,
            Some("srv".into()),
            Some("ui://ok-cache".into()),
            None,
        )
        .await
        .unwrap();
        assert_eq!(out.value["html"], "<p/>");
    }
    assert_eq!(port.reads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn inline_read_checks_owner() {
    let id = put_inline(InlineEntry {
        server_id: Some("srv".into()),
        resource: UiResource {
            html: "<i/>".into(),
            mime_type: "text/html".into(),
            csp: UiCsp::default(),
            permissions: None,
            prefers_border: false,
        },
    });
    let port = FakePort::default();
    let out = resource_read(&port, Some("srv".into()), None, Some(id.clone()))
        .await
        .unwrap();
    assert_eq!(out.value["html"], "<i/>");
    assert!(resource_read(&port, Some("other".into()), None, Some(id))
        .await
        .is_err());
    assert!(resource_read(&port, None, None, Some("missing".into()))
        .await
        .is_err());
}

#[tokio::test]
async fn tool_call_requires_confirmation_unless_read_only() {
    let port = FakePort::default();
    let policy = SecurityPolicy::default();
    let out = tool_call(
        &port,
        &policy,
        Some("srv".into()),
        Some("known".into()),
        None,
        false,
    )
    .await
    .unwrap();
    assert_eq!(out.value["requires_confirmation"], true);
    assert_eq!(port.calls.load(Ordering::SeqCst), 0);

    let out = tool_call(
        &port,
        &policy,
        Some("srv".into()),
        Some("known".into()),
        Some(json!({"a": 1})),
        true,
    )
    .await
    .unwrap();
    assert_eq!(out.value["requires_confirmation"], false);
    assert_eq!(out.value["result"]["structuredContent"], json!({"a": 1}));
    assert_eq!(port.calls.load(Ordering::SeqCst), 1);

    let port = FakePort {
        descriptor: Some(read_only()),
        ..FakePort::default()
    };
    let out = tool_call(
        &port,
        &policy,
        Some("srv".into()),
        Some("known".into()),
        None,
        false,
    )
    .await
    .unwrap();
    assert_eq!(out.value["read_only"], true);
    assert_eq!(port.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn tool_call_rejects_unknown_hidden_and_cross_server() {
    let policy = SecurityPolicy::default();
    let port = FakePort::default();
    assert!(tool_call(
        &port,
        &policy,
        Some("other".into()),
        Some("known".into()),
        None,
        true
    )
    .await
    .is_err());
    assert!(tool_call(
        &port,
        &policy,
        Some("srv".into()),
        Some("unknown".into()),
        None,
        true
    )
    .await
    .is_err());
    assert!(tool_call(
        &port,
        &policy,
        Some("srv".into()),
        Some("known".into()),
        Some(json!([1])),
        true
    )
    .await
    .is_err());

    let hidden = FakePort {
        descriptor: Some(UiToolDescriptor {
            meta: Some(json!({"ui": {"visibility": ["model"]}})),
            annotations: Some(json!({"readOnlyHint": true})),
        }),
        ..FakePort::default()
    };
    let err = tool_call(
        &hidden,
        &policy,
        Some("srv".into()),
        Some("known".into()),
        None,
        true,
    )
    .await
    .unwrap_err();
    assert!(err.contains("widget"));
    assert_eq!(hidden.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn tool_call_honours_policy_denial() {
    let port = FakePort::default();
    let policy = SecurityPolicy {
        autonomy: AutonomyLevel::ReadOnly,
        ..SecurityPolicy::default()
    };
    let err = tool_call(
        &port,
        &policy,
        Some("srv".into()),
        Some("known".into()),
        None,
        true,
    )
    .await
    .unwrap_err();
    assert!(err.contains("read-only"));
    assert_eq!(port.calls.load(Ordering::SeqCst), 0);
}
