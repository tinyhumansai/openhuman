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
#[cfg(feature = "native-security-tests")]
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

// ---------------------------------------------------------------------------
// Agent tool destinations
// ---------------------------------------------------------------------------

#[test]
fn local_only_registers_no_destination_instead_of_open_public_egress() {
    let policy = open_public_policy(subject(), tinysecurity_bus::PrivacyMode::LocalOnly);
    assert!(!policy.open_public);
    assert!(policy.destinations.is_empty() && policy.local_capabilities.is_empty());
}

#[test]
fn the_browser_policy_adds_only_a_browser_preflight() {
    let policy = browser_preflight_policy(subject(), tinysecurity_bus::PrivacyMode::Standard);
    assert!(policy.open_public);
    assert_eq!(policy.preflight_transports, vec![EgressTransport::Browser]);
    assert!(policy
        .data_categories
        .contains(&tinysecurity_bus::DataCategory::OpaqueBrowserTraffic));
    assert!(policy.destinations.is_empty() && policy.local_capabilities.is_empty());
}

#[test]
fn the_open_public_policy_registers_no_destination_and_no_local_capability() {
    let policy = open_public_policy(subject(), tinysecurity_bus::PrivacyMode::Standard);
    assert!(policy.open_public);
    assert!(policy.destinations.is_empty() && policy.local_capabilities.is_empty());
    assert!(policy.preflight_transports.is_empty());
}

#[test]
fn a_tool_request_carries_the_real_method_headers_and_body_as_user_content() {
    let headers = vec![("Authorization".to_string(), "Bearer abc".to_string())];
    let destination = ToolDestination {
        url: "https://example.test/x?q=1",
        method: "post",
        headers: &headers,
        body: Some("hello"),
    };
    let request = tool_request(EgressPolicyId("p".into()), subject(), &destination);
    assert_eq!(request.method, "POST");
    assert_eq!(request.payload, "hello");
    assert_eq!(request.headers.len(), 1);
    assert_eq!(request.category, DataCategory::UserContent);
    assert_eq!(request.transport, EgressTransport::PinnedDirect);
    assert!(request.local_capability.is_none() && request.transformation.is_none());
}

async fn register_open_public(
    proxy: &tinybus::Proxy,
    privacy: tinysecurity_bus::PrivacyMode,
) -> EgressPolicyId {
    proxy
        .call_confidential(
            names::methods::REGISTER_EGRESS_POLICY,
            (open_public_policy(subject(), privacy),),
        )
        .await
        .expect("the trusted host registers an open-public policy")
}

async fn judge(
    proxy: &tinybus::Proxy,
    id: &EgressPolicyId,
    url: &str,
    body: Option<&str>,
) -> EgressOutcome {
    let destination = ToolDestination {
        url,
        method: "POST",
        headers: &[],
        body,
    };
    let mut request = tool_request(id.clone(), subject(), &destination);
    request.tier = AccessTier::Full;
    scoped_url_guard_on(proxy, request)
        .await
        .expect("the module answers")
}

/// The released module, through the exact request the network tools' gate
/// builds: a public destination is authorized and pinned to its address, the
/// grant is spent once, and the address floor refuses everything private.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn released_module_authorizes_public_tool_destinations_and_refuses_private_ones() {
    let fx = super::super::native_tests::native_fixture().await;
    let standard = register_open_public(&fx.proxy, tinysecurity_bus::PrivacyMode::Standard).await;

    let public = "http://93.184.216.34/x";
    let EgressOutcome::Authorized(auth) = judge(&fx.proxy, &standard, public, Some("hi")).await
    else {
        panic!("a public literal must be authorized");
    };
    assert_eq!(auth.addresses, vec!["93.184.216.34:80".parse().unwrap()]);
    let destination = ToolDestination {
        url: public,
        method: "POST",
        headers: &[],
        body: Some("hi"),
    };
    let mut request = tool_request(standard.clone(), subject(), &destination);
    request.tier = AccessTier::Full;
    consume_on(&fx.proxy, request.clone(), (*auth).clone())
        .await
        .expect("the first use of a grant is accepted");
    assert!(
        consume_on(&fx.proxy, request, *auth).await.is_err(),
        "a grant is spent once"
    );

    for private in [
        "http://127.0.0.1/x",
        "http://169.254.169.254/latest/meta-data",
        "http://10.0.0.5/x",
        "http://100.64.0.1/x",
        "http://192.168.1.1/x",
    ] {
        assert!(
            matches!(
                judge(&fx.proxy, &standard, private, None).await,
                EgressOutcome::Denied(EgressDenial::Address)
            ),
            "{private} must hit the address floor"
        );
    }
}

/// Under Sensitive the module itself refuses a payload the redactor would
/// change, and under LocalOnly it refuses every external destination.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn released_module_applies_the_privacy_mode_to_the_tools_real_payload() {
    let fx = super::super::native_tests::native_fixture().await;
    let sensitive = register_open_public(&fx.proxy, tinysecurity_bus::PrivacyMode::Sensitive).await;
    // Built at runtime so no credential-shaped literal sits in the source.
    let secret = format!("{}-{}", "sk", "abcdefghijklmnopqrstuvwxyz123456");
    // A hostname, not an IP literal: under Sensitive the redactor treats an IP
    // address in the URL as an identifier, which is correct but not what this
    // case is about. Privacy is checked before any DNS, so a refused payload
    // never causes a lookup; a clean one passes the privacy check and may then
    // be denied only by the resolver of the machine running the test.
    let url = "http://example.test/x";
    assert!(matches!(
        judge(&fx.proxy, &sensitive, url, Some(&secret)).await,
        EgressOutcome::Denied(EgressDenial::Privacy)
    ));
    assert!(!matches!(
        judge(&fx.proxy, &sensitive, url, Some("plain words")).await,
        EgressOutcome::Denied(EgressDenial::Privacy)
    ));
    assert!(matches!(
        judge(&fx.proxy, &sensitive, "http://93.184.216.34/x", None).await,
        EgressOutcome::Denied(EgressDenial::Privacy)
    ));
    let local_only =
        register_open_public(&fx.proxy, tinysecurity_bus::PrivacyMode::LocalOnly).await;
    assert!(matches!(
        judge(&fx.proxy, &local_only, "http://93.184.216.34/x", None).await,
        EgressOutcome::Denied(_)
    ));
}

/// A browser navigation is judged as a verdict for the `Browser` transport: the
/// address floor still refuses private destinations, and no grant is spent.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn released_module_gives_browser_navigations_a_preflight_verdict() {
    let fx = super::super::native_tests::native_fixture().await;
    let id: EgressPolicyId = fx
        .proxy
        .call_confidential(
            names::methods::REGISTER_EGRESS_POLICY,
            (browser_preflight_policy(
                subject(),
                tinysecurity_bus::PrivacyMode::Standard,
            ),),
        )
        .await
        .expect("the trusted host registers a browser preflight policy");
    let verdict = |url: &'static str| {
        let destination = ToolDestination {
            url,
            method: "GET",
            headers: &[],
            body: None,
        };
        let mut request = tool_request(id.clone(), subject(), &destination);
        request.transport = EgressTransport::Browser;
        request
    };
    assert!(matches!(
        scoped_url_guard_on(&fx.proxy, verdict("https://93.184.216.34/page"))
            .await
            .expect("the module answers"),
        EgressOutcome::Authorized(_)
    ));
    for private in [
        "https://127.0.0.1/",
        "https://169.254.169.254/",
        "https://10.1.2.3/",
    ] {
        assert!(
            matches!(
                scoped_url_guard_on(&fx.proxy, verdict(private))
                    .await
                    .expect("the module answers"),
                EgressOutcome::Denied(EgressDenial::Address)
            ),
            "{private}"
        );
    }
}
