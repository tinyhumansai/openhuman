//! A loopback Hermes index and `SKILL.md` host for registry tests.

use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde_json::{json, Value};
use tinyskills::{Clock, FetchPolicy, HermesIndexSource, SkillRegistry};

use super::registry::RegistryConfig;
use super::transport::ReqwestTransport;

#[derive(Clone)]
pub(crate) struct Fixture {
    pub(crate) base: String,
    pub(crate) catalog_hits: Arc<AtomicUsize>,
    pub(crate) catalog_status: Arc<AtomicU16>,
    pub(crate) document_status: Arc<AtomicU16>,
    pub(crate) document_hits: Arc<AtomicUsize>,
    /// How many of the next `SKILL.md` responses carry content the
    /// supply-chain scan blocks.
    pub(crate) blocked_documents: Arc<AtomicUsize>,
    /// Changes the text of a blocked document, and so its digest.
    pub(crate) blocked_variant: Arc<AtomicUsize>,
}

#[derive(Clone)]
struct FixtureState {
    catalog: Arc<Mutex<Value>>,
    catalog_hits: Arc<AtomicUsize>,
    catalog_status: Arc<AtomicU16>,
    document_status: Arc<AtomicU16>,
    document_hits: Arc<AtomicUsize>,
    blocked_documents: Arc<AtomicUsize>,
    blocked_variant: Arc<AtomicUsize>,
}

/// A zero-width space: an invisible code point the scan blocks on.
pub(crate) const SCAN_BLOCKING_TEXT: &str = "Run the steps\u{200b} in order.";

async fn catalog_route(State(state): State<FixtureState>) -> Response {
    state.catalog_hits.fetch_add(1, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(30)).await;
    let status = state.catalog_status.load(Ordering::SeqCst);
    if status != 200 {
        return StatusCode::from_u16(status).unwrap().into_response();
    }
    axum::Json(state.catalog.lock().unwrap().clone()).into_response()
}

async fn document_route(
    State(state): State<FixtureState>,
    AxumPath(name): AxumPath<String>,
) -> Response {
    state.document_hits.fetch_add(1, Ordering::SeqCst);
    let status = state.document_status.load(Ordering::SeqCst);
    if status != 200 {
        let mut response = StatusCode::from_u16(status).unwrap().into_response();
        if status == 429 {
            response
                .headers_mut()
                .insert("retry-after", "42".parse().unwrap());
        }
        return response;
    }
    let blocked = state
        .blocked_documents
        .try_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
            left.checked_sub(1)
        })
        .is_ok();
    let body = if blocked {
        let variant = state.blocked_variant.load(Ordering::SeqCst);
        format!("{SCAN_BLOCKING_TEXT} Variant {variant}.")
    } else {
        String::new()
    };
    format!("---\nname: {name}\ndescription: Fixture skill {name}.\n---\n\n# {name}\n{body}\n")
        .into_response()
}

pub(crate) fn hermes_item(name: &str, source: &str) -> Value {
    json!({
        "name": name,
        "description": format!("{name} helper"),
        "category": "productivity",
        "source": source,
        "tags": ["fixture"],
        "docsPath": format!("fixture/productivity/{name}"),
    })
}

impl Fixture {
    pub(crate) async fn start(items: Vec<Value>) -> Self {
        let state = FixtureState {
            catalog: Arc::new(Mutex::new(Value::Array(items))),
            catalog_hits: Arc::new(AtomicUsize::new(0)),
            catalog_status: Arc::new(AtomicU16::new(200)),
            document_status: Arc::new(AtomicU16::new(200)),
            document_hits: Arc::new(AtomicUsize::new(0)),
            blocked_documents: Arc::new(AtomicUsize::new(0)),
            blocked_variant: Arc::new(AtomicUsize::new(0)),
        };
        let app = Router::new()
            .route("/skills.json", get(catalog_route))
            .route("/skills/{name}/SKILL.md", get(document_route))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            base: format!("http://{addr}"),
            catalog_hits: state.catalog_hits,
            catalog_status: state.catalog_status,
            document_status: state.document_status,
            document_hits: state.document_hits,
            blocked_documents: state.blocked_documents,
            blocked_variant: state.blocked_variant,
        }
    }

    pub(crate) fn registry(&self) -> Arc<SkillRegistry> {
        self.build(tinyskills::SystemClock, true)
    }

    pub(crate) fn registry_without_download_base(&self) -> Arc<SkillRegistry> {
        self.build(tinyskills::SystemClock, false)
    }

    pub(crate) fn registry_with_clock(&self, clock: impl Clock + 'static) -> Arc<SkillRegistry> {
        self.build(clock, true)
    }

    fn build(&self, clock: impl Clock + 'static, download_base: bool) -> Arc<SkillRegistry> {
        let mut policy = FetchPolicy::default();
        policy.allow_loopback_http = true;
        let source = HermesIndexSource::new("hermes", format!("{}/skills.json", self.base));
        let source = if download_base {
            source.with_download_base(format!("{}/skills", self.base))
        } else {
            source
        };
        SkillRegistry::builder(ReqwestTransport::new())
            .source(source)
            .policy(policy)
            .timeouts(RegistryConfig::timeouts())
            .limits(RegistryConfig::limits())
            .clock(clock)
            .build()
    }
}
