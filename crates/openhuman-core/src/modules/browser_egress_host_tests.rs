use super::*;
use tinybus::{broker::Broker, service::Interface, transport::memory::MemoryBus, Connection};
use tinycomputer_bus::consequence_host::{self, ConsequenceKind, ReviewActionRequest};

#[test]
fn browser_policy_cache_refreshes_before_the_policy_expires() {
    let registered_at = std::time::Instant::now();
    let scopes = vec![(
        "policy-1".to_owned(),
        7,
        "user-1".to_owned(),
        "workspace-1".to_owned(),
        "agent-1".to_owned(),
        "Standard".to_owned(),
        registered_at,
    )];
    let matching_scope = (7, "user-1", "workspace-1", "agent-1", "Standard");

    let reusable = find_reusable_policy(
        &scopes,
        matching_scope.0,
        matching_scope.1,
        matching_scope.2,
        matching_scope.3,
        matching_scope.4,
        registered_at + std::time::Duration::from_secs(24),
    );
    assert_eq!(reusable, Some(("policy-1", 7)));

    let expired_from_cache = find_reusable_policy(
        &scopes,
        matching_scope.0,
        matching_scope.1,
        matching_scope.2,
        matching_scope.3,
        matching_scope.4,
        registered_at + BROWSER_POLICY_REFRESH_AFTER,
    );
    assert!(
        expired_from_cache.is_none(),
        "policy registration refreshes before its 30 second TinySecurity validity ends"
    );
}

fn target(policy_id: &str) -> BrowserEgressTarget {
    BrowserEgressTarget {
        policy_id: policy_id.into(),
        expected_generation: 1,
        session_id: "browser-session-test".into(),
        scheme: "https".into(),
        hostname: "8.8.8.8".into(),
        port: 443,
    }
}

async fn host_bus() -> (
    MemoryBus,
    Connection,
    tokio::task::JoinHandle<tinybus::Result<()>>,
) {
    let transport = MemoryBus::new();
    let broker = Broker::new();
    let task = broker.spawn(transport.clone());
    let host = Connection::connect(transport.connect().await.unwrap())
        .await
        .unwrap();
    install(&host).await.unwrap();
    (transport, host, task)
}

#[tokio::test]
async fn browser_egress_callback_rejects_unauthenticated_peers() {
    let (transport, _host, _broker) = host_bus().await;
    let caller = Connection::connect(transport.connect().await.unwrap())
        .await
        .unwrap();
    let proxy = caller
        .proxy(
            contract::INTERFACE,
            contract::OBJECT_PATH,
            contract::INTERFACE,
        )
        .unwrap();
    let result: tinybus::Result<BrowserEgressGrant> = proxy
        .call(contract::methods::AUTHORIZE, (target("unregistered"),))
        .await;
    assert!(result.is_err());
}

#[test]
fn consequence_review_requires_confidential_delivery() {
    let host = super::ConsequenceReviewHost;
    let review = consequence_host::methods::REVIEW.parse().unwrap();
    assert!(host.requires_confidential(&review));
}

#[tokio::test]
async fn consequence_review_denies_when_the_approval_gate_is_unavailable() {
    let response = super::ConsequenceReviewHost
        .review(ReviewActionRequest {
            task: "task-1".into(),
            action: "submit".into(),
            target: "button".into(),
            kind: ConsequenceKind::Irreversible,
            host_context: Some(tinycomputer_bus::agent::HostContext::new(
                "opaque-host-context",
            )),
        })
        .await
        .expect("missing approval gate is a deny response");

    assert!(!response.allow);
    assert_eq!(
        response.reason.as_deref(),
        Some("Approval gate is unavailable")
    );
}

#[tokio::test]
async fn browser_egress_callback_refuses_unknown_host_registered_scope() {
    let (transport, _host, _broker) = host_bus().await;
    let caller = Connection::connect(transport.connect().await.unwrap())
        .await
        .unwrap();
    caller
        .request_name(tinycomputer_bus::names::INTERFACE)
        .await
        .unwrap();
    let proxy = caller
        .proxy(
            contract::INTERFACE,
            contract::OBJECT_PATH,
            contract::INTERFACE,
        )
        .unwrap();
    let result: tinybus::Result<BrowserEgressGrant> = proxy
        .call(contract::methods::AUTHORIZE, (target("unregistered"),))
        .await;
    assert!(result.is_err());
}
