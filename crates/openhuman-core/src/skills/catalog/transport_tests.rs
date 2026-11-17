use super::*;

use std::sync::Arc;

use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use tinyskills::{
    fetch_skill_document, FetchPolicy, RegistryError, RegistryLimits, RegistryTimeouts,
    SystemResolver,
};

const SKILL_MD: &str =
    "---\nname: pinned-skill\ndescription: Served over loopback.\n---\n\n# Pinned\n";

async fn serve(app: Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

fn fixture() -> Router {
    Router::new()
        .route("/SKILL.md", get(|| async { SKILL_MD }))
        .route(
            "/moved.md",
            get(|| async {
                (
                    StatusCode::MOVED_PERMANENTLY,
                    [("location", "/SKILL.md")],
                    "",
                )
            }),
        )
        .route("/unchanged", get(|| async { StatusCode::NOT_MODIFIED }))
        .route(
            "/echo",
            get(|headers: HeaderMap| async move {
                headers
                    .get("x-registry-probe")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("missing")
                    .to_owned()
                    .into_response()
            }),
        )
}

async fn read_body(response: &mut TransportResponse) -> Vec<u8> {
    let mut body = Vec::new();
    while let Some(chunk) = response.body.next_chunk().await.unwrap() {
        body.extend_from_slice(&chunk);
    }
    body
}

async fn get_from(
    transport: &ReqwestTransport,
    url: String,
    pinned: SocketAddr,
    headers: &[(String, String)],
) -> TransportResponse {
    transport
        .exchange(
            HttpMethod::Get,
            url,
            &[pinned],
            headers,
            Duration::from_secs(5),
        )
        .await
        .expect("exchange")
}

#[tokio::test]
async fn connects_to_the_pinned_address_not_the_url_host() {
    let addr = serve(fixture()).await;
    let transport = ReqwestTransport::new();
    let url = format!("http://registry.invalid:{}/SKILL.md", addr.port());
    let mut response = get_from(&transport, url.clone(), addr, &[]).await;
    assert_eq!(response.status, 200);
    assert_eq!(response.final_url, url);
    assert_eq!(read_body(&mut response).await, SKILL_MD.as_bytes());
}

#[tokio::test]
async fn returns_a_redirect_without_following_it() {
    let addr = serve(fixture()).await;
    let transport = ReqwestTransport::new();
    let url = format!("http://127.0.0.1:{}/moved.md", addr.port());
    let response = get_from(&transport, url.clone(), addr, &[]).await;
    assert_eq!(response.status, 301);
    assert_eq!(response.header("location"), Some("/SKILL.md"));
    assert_eq!(response.final_url, url, "final_url is the requested url");
}

#[tokio::test]
async fn passes_a_not_modified_answer_through() {
    let addr = serve(fixture()).await;
    let transport = ReqwestTransport::new();
    let url = format!("http://127.0.0.1:{}/unchanged", addr.port());
    let mut response = get_from(&transport, url, addr, &[]).await;
    assert_eq!(response.status, 304);
    assert!(read_body(&mut response).await.is_empty());
}

#[tokio::test]
async fn sends_every_request_header() {
    let addr = serve(fixture()).await;
    let transport = ReqwestTransport::new();
    let url = format!("http://127.0.0.1:{}/echo", addr.port());
    let headers = vec![("X-Registry-Probe".to_owned(), "present".to_owned())];
    let mut response = get_from(&transport, url, addr, &headers).await;
    assert_eq!(read_body(&mut response).await, b"present");
}

#[tokio::test]
async fn head_requests_carry_no_body() {
    let addr = serve(fixture()).await;
    let transport = ReqwestTransport::new();
    let url = format!("http://127.0.0.1:{}/SKILL.md", addr.port());
    let mut response = transport
        .exchange(HttpMethod::Head, url, &[addr], &[], Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(response.status, 200);
    assert!(read_body(&mut response).await.is_empty());
}

#[tokio::test]
async fn an_unreachable_pinned_address_is_a_connect_error() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let transport = ReqwestTransport::new();
    let Err(error) = transport
        .exchange(
            HttpMethod::Get,
            format!("http://127.0.0.1:{}/SKILL.md", addr.port()),
            &[addr],
            &[],
            Duration::from_secs(5),
        )
        .await
    else {
        panic!("nothing listens there");
    };
    assert!(matches!(error, TransportError::Connect(_)), "{error:?}");
}

fn loopback_policy() -> FetchPolicy {
    let mut policy = FetchPolicy::default();
    policy.allow_loopback_http = true;
    policy
}

async fn fetch_through_guard(
    url: String,
    timeouts: RegistryTimeouts,
    limits: RegistryLimits,
) -> Result<tinyskills::RegistryDocument, RegistryError> {
    fetch_skill_document(
        Arc::new(ReqwestTransport::new()),
        Arc::new(SystemResolver),
        &url,
        &loopback_policy(),
        &timeouts,
        &limits,
    )
    .await
}

#[tokio::test]
async fn the_guard_follows_a_redirect_through_the_transport() {
    let addr = serve(fixture()).await;
    let document = fetch_through_guard(
        format!("http://127.0.0.1:{}/moved.md", addr.port()),
        RegistryTimeouts::default(),
        RegistryLimits::default(),
    )
    .await
    .expect("document");
    assert_eq!(document.document.slug, "pinned-skill");
}

#[tokio::test]
async fn an_oversized_body_stops_the_stream() {
    let app = Router::new().route("/SKILL.md", get(|| async { "x".repeat(64 * 1024) }));
    let addr = serve(app).await;
    let mut limits = RegistryLimits::default();
    limits.max_document_bytes = 1024;
    let error = fetch_through_guard(
        format!("http://127.0.0.1:{}/SKILL.md", addr.port()),
        RegistryTimeouts::default(),
        limits,
    )
    .await
    .expect_err("body exceeds the limit");
    assert!(matches!(error, RegistryError::TooLarge { .. }), "{error:?}");
}

#[tokio::test]
async fn a_slow_upstream_times_out() {
    let app = Router::new().route(
        "/SKILL.md",
        get(|| async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            SKILL_MD
        }),
    );
    let addr = serve(app).await;
    let mut timeouts = RegistryTimeouts::default();
    timeouts.document = Duration::from_millis(200);
    let error = fetch_through_guard(
        format!("http://127.0.0.1:{}/SKILL.md", addr.port()),
        timeouts,
        RegistryLimits::default(),
    )
    .await
    .expect_err("slow upstream");
    assert!(error.is_timeout(), "{error:?}");
}

#[test]
fn clients_are_reused_per_pinned_host() {
    let transport = ReqwestTransport::new();
    let pinned: SocketAddr = "127.0.0.1:443".parse().unwrap();
    transport
        .client_for("a.example", &[pinned], Duration::from_secs(5))
        .unwrap();
    transport
        .client_for("a.example", &[pinned], Duration::from_secs(5))
        .unwrap();
    transport
        .client_for("b.example", &[pinned], Duration::from_secs(5))
        .unwrap();
    assert_eq!(transport.clients.lock().unwrap().len(), 2);
}
