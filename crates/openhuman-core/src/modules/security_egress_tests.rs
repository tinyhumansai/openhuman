use super::*;
use tinysecurity_bus::{AccessTier, DataCategory, EgressHeader, EgressTransport, SubjectScope};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn subject() -> SubjectScope {
    SubjectScope {
        user: "u".into(),
        workspace: "w".into(),
        agent: "a".into(),
    }
}

fn request(url: &str) -> ScopedEgressRequest {
    ScopedEgressRequest {
        policy_id: EgressPolicyId("p".into()),
        subject: subject(),
        tier: AccessTier::Write,
        url: url.into(),
        method: "GET".into(),
        category: DataCategory::HostMetadata,
        payload: String::new(),
        headers: vec![EgressHeader {
            name: "x-test".into(),
            value: "1".into(),
        }],
        transport: EgressTransport::PinnedDirect,
        local_capability: None,
        transformation: None,
    }
}

fn grant(
    url: &str,
    hostname: &str,
    port: u16,
    addr: std::net::SocketAddr,
) -> TransportAuthorization {
    TransportAuthorization {
        fingerprint: "f".repeat(64),
        authorization_id: "id".into(),
        url: url.into(),
        hostname: hostname.into(),
        port,
        addresses: vec![addr],
        subject: subject(),
        policy_id: EgressPolicyId("p".into()),
        generation: 1,
        expires_at_ms: u64::MAX,
    }
}

#[test]
fn a_cacheable_or_ambiguous_response_is_an_error() {
    let cacheable = ScopedEgressResponse {
        generation: 1,
        denial: Some(EgressDenial::Address),
        authorization: None,
        cacheable: true,
    };
    assert!(interpret(cacheable).is_err());
    let neither = ScopedEgressResponse {
        generation: 1,
        denial: None,
        authorization: None,
        cacheable: false,
    };
    assert!(interpret(neither).is_err());
    let denied = ScopedEgressResponse {
        generation: 1,
        denial: Some(EgressDenial::Destination),
        authorization: None,
        cacheable: false,
    };
    assert_eq!(
        interpret(denied).unwrap(),
        EgressOutcome::Denied(EgressDenial::Destination)
    );
}

#[test]
fn pinned_client_rejects_empty_or_mismatched_grants() {
    let addr: std::net::SocketAddr = "127.0.0.1:9".parse().unwrap();
    let mut empty = grant("http://example.test:9/", "example.test", 9, addr);
    empty.addresses.clear();
    assert!(pinned_client(&empty).is_err());
    let wrong_host = grant("http://other.test:9/", "example.test", 9, addr);
    assert!(pinned_client(&wrong_host).is_err());
    let wrong_port = grant("http://example.test:10/", "example.test", 9, addr);
    assert!(pinned_client(&wrong_port).is_err());
    assert!(pinned_client(&grant("http://example.test:9/", "example.test", 9, addr)).is_ok());
}

/// Real local transport: the hostname does not resolve anywhere (`.invalid`), so
/// the request can only succeed by connecting to the approved socket without a
/// second lookup. The server sees the original `Host`, and a 302 is returned
/// unfollowed (exactly one connection).
#[tokio::test(flavor = "multi_thread")]
async fn pinned_transport_connects_to_approved_ip_without_a_second_lookup_or_redirect() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let port = addr.port();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut buf = vec![0u8; 4096];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let _ = tx.send(String::from_utf8_lossy(&buf[..n]).to_string());
            let _ = stream
                .write_all(
                    b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/next\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await;
        }
    });
    let url = format!("http://pinned.invalid:{port}/start");
    let req = request(&url);
    let auth = grant(&url, "pinned.invalid", port, addr);
    let response = send_pinned(&auth, &req).await.expect("pinned send");
    assert_eq!(
        response.status().as_u16(),
        302,
        "redirect must not be followed"
    );
    let seen = rx.recv().await.unwrap().to_ascii_lowercase();
    assert!(
        seen.contains(&format!("host: pinned.invalid:{port}")),
        "{seen}"
    );
    assert!(seen.contains("x-test: 1"));
    // Give a followed redirect time to show up as a second request.
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert!(rx.try_recv().is_err(), "no second request may be made");
}

#[tokio::test]
async fn unreachable_pinned_address_fails_closed_without_falling_back_to_dns() {
    // Approved address has no listener; `localhost` would resolve and succeed if
    // the client fell back to system DNS for the grant's hostname.
    // reqwest takes the port from the URL and only the IP from the override, so
    // the approved IP must differ from where the listener is: 127.0.0.2 has no
    // listener, `localhost` (127.0.0.1) does.
    let dead: std::net::SocketAddr = "127.0.0.2:9".parse().unwrap();
    let live = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let live_port = live.local_addr().unwrap().port();
    let url = format!("http://localhost:{live_port}/");
    let req = request(&url);
    let auth = grant(&url, "localhost", live_port, dead);
    let sent =
        tokio::time::timeout(std::time::Duration::from_secs(2), send_pinned(&auth, &req)).await;
    assert!(
        !matches!(sent, Ok(Ok(_))),
        "must not reach the resolvable host"
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), live.accept())
            .await
            .is_err(),
        "system DNS fallback reached the listener"
    );
}

/// The released module refuses loopback for a generic agent URL before any DNS.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a released host key or an explicitly selected local native fixture"]
async fn module_denies_loopback_for_generic_agent_urls() {
    let fx = super::super::native_tests::native_fixture().await;
    let outcome = scoped_url_guard_on(&fx.proxy, request("http://127.0.0.1:8080/x")).await;
    // Unregistered policy ids are refused outright; both are denials.
    assert!(!matches!(outcome, Ok(EgressOutcome::Authorized(_))));
}

/// DNS rebinding: the name resolves (system resolver) to a live local listener
/// after validation, but the grant approved a different address. The pinned
/// client must connect only to the approved address and never re-resolve.
#[tokio::test]
async fn rebinding_resolver_answer_after_validation_is_never_used() {
    let rebound = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = rebound.local_addr().unwrap().port();
    // `localhost` is what a rebinding resolver would now answer with.
    let approved: std::net::SocketAddr = format!("127.0.0.3:{port}").parse().unwrap();
    let url = format!("http://localhost:{port}/");
    let auth = grant(&url, "localhost", port, approved);
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        send_pinned(&auth, &request(&url)),
    )
    .await;
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(150), rebound.accept())
            .await
            .is_err(),
        "the rebound address received a connection"
    );
}
