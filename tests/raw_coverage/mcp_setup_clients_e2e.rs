//! JSON-RPC E2E coverage for the MCP guided-setup surface (`mcp_setup_*`), the
//! four uncovered `mcp_clients_*` controllers, and `mcp_audit_list`.
//!
//! Run:
//! `cargo test --test raw_coverage_all --features "$(bash scripts/ci/product-features.sh)" mcp_setup_clients_e2e`
//!
//! ## The registry is a loopback fixture, not the live catalog
//!
//! `mcp_setup_search` / `mcp_setup_get` / `mcp_clients_registry_get` read the
//! official `modelcontextprotocol/registry` — `GET /v0/servers` to list, `GET
//! /v0/servers/{name}/versions` for detail
//! (`vendor/tinymcp/.../registry/sources/official/mod.rs:1-6`). Its base is
//! overridable via `MCP_OFFICIAL_REGISTRY_BASE` (`:353-360`), so this suite
//! stands up an axum server speaking that shape and points the resolver at it.
//! Nothing leaves the machine.
//!
//! Smithery, the second source, stays off: it is opt-in on an API key and this
//! suite configures none, so search resolves the official source alone and the
//! result set is exactly the fixture's (`sources/mod.rs:9-19`).
//!
//! ## The secret vault is driven through both of its RPCs at once
//!
//! `mcp_setup_request_secret` **blocks** until a UI answers — up to five
//! minutes — and only then returns the ref. The ref reaches the UI over the
//! event bus as `McpSetupSecretRequested`, so the pair can only be exercised
//! together: this suite initialises the in-process bus, subscribes, issues
//! `request_secret` on one task, reads the ref off the bus, and fulfils it with
//! `mcp_setup_submit_secret` from another. That is the real shape of the flow,
//! not a simulation of it.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::extract::Path as AxumPath;
use axum::http::header::AUTHORIZATION;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

use openhuman_core::core::auth::{init_rpc_token, CORE_TOKEN_ENV_VAR};
use openhuman_core::core::events::DomainEvent;
use openhuman_core::core::jsonrpc::build_core_http_router;

/// The bearer this suite *proposes*. It is only used if this suite happens to
/// be the first in the aggregated binary to initialise the token subsystem —
/// see `rpc_token()` below, which is what actually gets sent.
const PROPOSED_RPC_TOKEN: &str = "mcp-setup-clients-e2e-token";

static AUTH_INIT: OnceLock<()> = OnceLock::new();

/// The crate-wide env lock, not a private one.
static ENV_LOCK: &OnceLock<Mutex<()>> = &crate::SHARED_ENV_LOCK;

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ── Env plumbing ────────────────────────────────────────────────────────────

struct EnvVarGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvVarGuard {
    fn set_to_path(key: &'static str, path: &Path) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, path.as_os_str());
        Self { key, old }
    }

    fn set(key: &'static str, value: &str) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, old }
    }

    fn unset(key: &'static str) -> Self {
        let old = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

/// The bearer the running process will actually validate.
///
/// `core::auth::RPC_TOKEN` is a process-global `OnceLock` and `init_rpc_token`
/// returns early once it is set. Every `tests/raw_coverage/` suite now shares
/// one process, so the FIRST suite to initialise pins the token for all of
/// them; a suite that sent its own hard-coded literal would get 401 whenever it
/// lost that race, in an order that depends on libtest scheduling and load.
///
/// So: propose a token, initialise, then ask what the process settled on and
/// send that. Correct whether this suite wins the race or loses it.
fn rpc_token() -> &'static str {
    AUTH_INIT.get_or_init(|| {
        if std::env::var(CORE_TOKEN_ENV_VAR)
            .map(|v| v.trim().is_empty())
            .unwrap_or(true)
        {
            std::env::set_var(CORE_TOKEN_ENV_VAR, PROPOSED_RPC_TOKEN);
        }
        let token_dir = std::env::temp_dir().join("openhuman-mcp-setup-clients-e2e-auth");
        std::fs::create_dir_all(&token_dir).expect("token dir");
        init_rpc_token(&token_dir).expect("init rpc auth token");
    });
    openhuman_core::core::auth::get_rpc_token()
        .expect("the token subsystem is initialised by the line above")
}

// ── Harness ─────────────────────────────────────────────────────────────────

const MIN_CONFIG: &str = r#"api_url = "http://127.0.0.1:9"
default_model = "mcp-e2e-model"

[secrets]
encrypt = false

[local_ai]
enabled = false

[memory]
provider = "none"
embedding_provider = "none"
embedding_model = "none"
embedding_dimensions = 0

[memory_tree]
embedding_strict = false
"#;

struct Harness {
    _tmp: TempDir,
    _guards: Vec<EnvVarGuard>,
    workspace: std::path::PathBuf,
    rpc_base: String,
    join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
}

async fn setup(extra: Vec<EnvVarGuard>) -> Harness {
    let _ = rpc_token();

    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");
    std::fs::create_dir_all(&openhuman_home).expect("create .openhuman");
    std::fs::write(openhuman_home.join("config.toml"), MIN_CONFIG).expect("write config.toml");
    let _: openhuman_core::openhuman::config::Config =
        toml::from_str(MIN_CONFIG).expect("test config must match the config schema");

    let workspace = home.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");

    let mut guards = vec![
        EnvVarGuard::set_to_path("HOME", home),
        EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace),
        EnvVarGuard::unset("BACKEND_URL"),
        EnvVarGuard::unset("VITE_BACKEND_URL"),
        EnvVarGuard::unset("OPENHUMAN_API_URL"),
        // No Smithery key: the second registry source stays off, so the fixture
        // below is the only catalog in play.
        EnvVarGuard::unset("SMITHERY_API_KEY"),
        EnvVarGuard::unset("MCP_OFFICIAL_REGISTRY_TOKEN"),
        EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
    ];
    guards.extend(extra);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind rpc listener");
    let addr: SocketAddr = listener.local_addr().expect("rpc listener addr");
    let router = build_core_http_router(false);
    let join = tokio::spawn(async move { axum::serve(listener, router).await });

    Harness {
        _tmp: tmp,
        _guards: guards,
        workspace,
        rpc_base: format!("http://{addr}"),
        join,
    }
}

async fn rpc(base: &str, id: i64, method: &str, params: Value) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .expect("build client");
    let url = format!("{}/rpc", base.trim_end_matches('/'));
    let response = client
        .post(&url)
        .header(AUTHORIZATION, format!("Bearer {}", rpc_token()))
        .json(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
        .send()
        .await
        .unwrap_or_else(|err| panic!("POST {url} {method}: {err}"));
    assert!(
        response.status().is_success(),
        "HTTP {} for {method}",
        response.status()
    );
    response
        .json::<Value>()
        .await
        .unwrap_or_else(|err| panic!("json for {method}: {err}"))
}

fn ok<'a>(value: &'a Value, context: &str) -> &'a Value {
    if let Some(error) = value.get("error") {
        panic!("{context}: unexpected JSON-RPC error: {error}");
    }
    value
        .get("result")
        .unwrap_or_else(|| panic!("{context}: missing result: {value}"))
}

fn error_message<'a>(value: &'a Value, context: &str) -> &'a str {
    value
        .get("error")
        .unwrap_or_else(|| panic!("{context}: expected a JSON-RPC error, got: {value}"))
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{context}: error carries no message: {value}"))
}

// ── The fixture registry ────────────────────────────────────────────────────

/// The two names the fixture catalog serves.
const INSTALLABLE: &str = "io.github.e2e/planted-server";
/// Declares neither a remote nor a package, so `is_installable()` is false and
/// the list adapter must drop it.
const UNINSTALLABLE: &str = "io.github.e2e/shell-only-row";

fn fixture_server_record(name: &str, installable: bool) -> Value {
    let mut record = json!({
        "name": name,
        "title": "Planted E2E Server",
        "description": "A planted MCP server record for the e2e suite.",
        "websiteUrl": "https://example.invalid/planted",
        "remotes": [],
        "packages": []
    });
    if installable {
        record["packages"] = json!([{
            "registryType": "npm",
            "identifier": "@e2e/planted-server",
            "environmentVariables": [
                {
                    "name": "PLANTED_API_KEY",
                    "description": "The credential the planted server needs.",
                    "isRequired": true,
                    "isSecret": true
                }
            ]
        }]);
    }
    record
}

/// An axum server speaking the official registry's list + versions shapes.
async fn serve_fixture_registry() -> (
    SocketAddr,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    async fn list() -> Json<Value> {
        Json(json!({
            "servers": [
                { "server": fixture_server_record(INSTALLABLE, true) },
                { "server": fixture_server_record(UNINSTALLABLE, false) }
            ],
            // Empty next cursor: the adapter treats it as absent, so the page
            // bound stays at the current page rather than paging forever.
            "metadata": { "nextCursor": "" }
        }))
    }

    async fn versions(AxumPath(name): AxumPath<String>) -> Json<Value> {
        if name == INSTALLABLE {
            Json(json!({
                "servers": [{ "server": fixture_server_record(INSTALLABLE, true) }]
            }))
        } else {
            // The registry lists no version of it — the adapter's
            // `Error::UnknownServer` path.
            Json(json!({ "servers": [] }))
        }
    }

    let app = Router::new()
        .route("/v0/servers", get(list))
        .route("/v0/servers/{name}/versions", get(versions));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture registry");
    let addr = listener.local_addr().expect("fixture registry addr");
    let join = tokio::spawn(async move { axum::serve(listener, app).await });
    (addr, join)
}

// ── mcp_setup: the catalog half ─────────────────────────────────────────────

/// `mcp_setup_search` and `mcp_setup_get` against the fixture registry.
#[tokio::test]
async fn mcp_setup_search_and_get_read_the_configured_registry() {
    let _lock = env_lock();
    let (registry_addr, registry_join) = serve_fixture_registry().await;
    let harness = setup(vec![EnvVarGuard::set(
        "MCP_OFFICIAL_REGISTRY_BASE",
        &format!("http://{registry_addr}"),
    )])
    .await;

    let found = rpc(
        &harness.rpc_base,
        200,
        "openhuman.mcp_setup_search",
        json!({ "query": "planted", "page": 1, "page_size": 10 }),
    )
    .await;
    let found = ok(&found, "mcp_setup_search");
    let servers = found
        .get("servers")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("search has no `servers` array: {found}"));

    // Exactly one row: the un-installable record must be filtered out rather
    // than shown as a dead end the user can only discover by trying.
    assert_eq!(
        servers.len(),
        1,
        "the row declaring neither a remote nor a package must be dropped: {found}"
    );
    let row = &servers[0];
    assert_eq!(
        row.get("qualified_name").and_then(Value::as_str),
        Some(INSTALLABLE),
        "the surviving row must be the installable one: {row}"
    );
    assert_eq!(
        row.get("source").and_then(Value::as_str),
        Some("mcp_official"),
        "every row must be tagged with the registry it came from: {row}"
    );
    assert_eq!(
        row.get("display_name").and_then(Value::as_str),
        Some("Planted E2E Server"),
        "the declared title must win over the derived one: {row}"
    );
    assert_eq!(
        row.get("auth_kind").and_then(Value::as_str),
        Some("api_key"),
        "a package declaring an `isSecret` env var must be badged as needing a key: {row}"
    );
    assert_eq!(
        found.get("page").and_then(Value::as_u64),
        Some(1),
        "the requested page must be echoed: {found}"
    );
    assert_eq!(
        found.get("total_pages").and_then(Value::as_u64),
        Some(1),
        "an empty nextCursor means this is the last page: {found}"
    );

    // `get` adds `required_env_keys`, which is the whole reason the setup
    // dialog calls it rather than reading the search row.
    let detail = rpc(
        &harness.rpc_base,
        201,
        "openhuman.mcp_setup_get",
        json!({ "qualified_name": INSTALLABLE }),
    )
    .await;
    let detail = ok(&detail, "mcp_setup_get");
    let server = detail
        .get("server")
        .unwrap_or_else(|| panic!("get has no `server`: {detail}"));
    assert_eq!(
        server.get("qualified_name").and_then(Value::as_str),
        Some(INSTALLABLE)
    );
    assert_eq!(
        server
            .get("required_env_keys")
            .and_then(Value::as_array)
            .map(|keys| keys.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
        Some(vec!["PLANTED_API_KEY"]),
        "the declared secret env var must be injected as a required key: {server}"
    );
    // The package became a stdio connection with a worked example.
    let connections = server
        .get("connections")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("detail has no `connections`: {server}"));
    assert_eq!(
        connections.len(),
        1,
        "one package, one connection: {server}"
    );
    assert_eq!(
        connections[0].get("type").and_then(Value::as_str),
        Some("stdio"),
        "an npm package must become a stdio connection: {server}"
    );

    // Failure path 1: a blank name is refused by the shared identifier guard,
    // before any network call.
    let blank = rpc(
        &harness.rpc_base,
        202,
        "openhuman.mcp_setup_get",
        json!({ "qualified_name": "   " }),
    )
    .await;
    assert!(
        error_message(&blank, "mcp_setup_get blank").contains("qualified_name must not be empty"),
        "a blank qualified_name must be named in the refusal: {blank}"
    );

    // Failure path 2: the param is absent entirely — a different error from a
    // blank one, and it must stay different.
    let absent = rpc(&harness.rpc_base, 203, "openhuman.mcp_setup_get", json!({})).await;
    assert!(
        error_message(&absent, "mcp_setup_get absent").contains("missing required param"),
        "an absent qualified_name must be a missing-param error: {absent}"
    );

    // Failure path 3: a name the registry lists no version of.
    let unknown = rpc(
        &harness.rpc_base,
        204,
        "openhuman.mcp_setup_get",
        json!({ "qualified_name": UNINSTALLABLE }),
    )
    .await;
    assert!(
        unknown.get("error").is_some(),
        "a name with no published version must error, not return an empty detail: {unknown}"
    );

    registry_join.abort();
    harness.join.abort();
}

// ── mcp_setup: the secret half ──────────────────────────────────────────────

/// `mcp_setup_request_secret` + `mcp_setup_submit_secret`, driven as the pair
/// they actually are.
///
/// `request_secret` blocks on the vault until a UI answers, publishing the ref
/// on the event bus on the way in. This case plays both halves: one task issues
/// the request, and the ref that comes back over the bus is what the other task
/// submits. Asserting only one half would assert nothing — the request never
/// returns without the submit, and the submit has no ref without the request.
#[tokio::test]
async fn mcp_setup_secret_request_blocks_until_the_matching_submit_lands() {
    let _lock = env_lock();
    let harness = setup(Vec::new()).await;
    openhuman_core::core::bus::init().await.expect("bus init");

    // Capture the ref the flow publishes for the UI.
    struct RefCatcher {
        tx: tokio::sync::mpsc::UnboundedSender<(String, String, String)>,
    }

    #[async_trait::async_trait]
    impl tinybus::EventHandler<DomainEvent> for RefCatcher {
        fn name(&self) -> &str {
            "mcp-setup-e2e::ref-catcher"
        }

        async fn handle(&self, event: &DomainEvent) {
            if let DomainEvent::McpSetupSecretRequested {
                ref_id,
                key_name,
                prompt,
            } = event
            {
                let _ = self
                    .tx
                    .send((ref_id.clone(), key_name.clone(), prompt.clone()));
            }
        }
    }

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let _subscription = openhuman_core::core::bus::BUS
        .subscribe(Arc::new(RefCatcher { tx }))
        .expect("the in-process bus accepts a subscriber once init() has run");

    let base = harness.rpc_base.clone();
    let request = tokio::spawn(async move {
        rpc(
            &base,
            210,
            "openhuman.mcp_setup_request_secret",
            json!({ "key_name": "PLANTED_API_KEY", "prompt": "Paste the planted key." }),
        )
        .await
    });

    let (ref_id, key_name, prompt) = tokio::time::timeout(Duration::from_secs(30), rx.recv())
        .await
        .expect("the request must publish its ref promptly")
        .expect("the ref-catcher channel stays open");
    assert_eq!(
        key_name, "PLANTED_API_KEY",
        "the published event must carry the requested key name"
    );
    assert_eq!(
        prompt, "Paste the planted key.",
        "the published event must carry the prompt the UI renders"
    );
    assert!(
        ref_id.starts_with("secret://"),
        "the minted handle must be an opaque `secret://` ref, got {ref_id}"
    );

    // The request is still blocked: nothing has answered it yet. Proving that
    // is the point — a `request_secret` that returned early would hand the
    // agent a ref with no value behind it.
    assert!(
        !request.is_finished(),
        "request_secret must not return before a value is submitted"
    );

    let submitted = rpc(
        &harness.rpc_base,
        211,
        "openhuman.mcp_setup_submit_secret",
        json!({ "ref_id": ref_id, "value": "planted-secret-value" }),
    )
    .await;
    let submitted = ok(&submitted, "mcp_setup_submit_secret");
    assert_eq!(
        submitted.get("ref").and_then(Value::as_str),
        Some(ref_id.as_str()),
        "submit must echo the ref it fulfilled: {submitted}"
    );
    assert_eq!(
        submitted.get("fulfilled").and_then(Value::as_bool),
        Some(true),
        "a first submit must report fulfilled: {submitted}"
    );

    // Now the blocked request completes, returning the ref — and never the
    // value, which is the security property the whole indirection exists for.
    let requested = tokio::time::timeout(Duration::from_secs(30), request)
        .await
        .expect("request_secret must return once the submit lands")
        .expect("request task must not panic");
    let requested = ok(&requested, "mcp_setup_request_secret");
    assert_eq!(
        requested.get("ref").and_then(Value::as_str),
        Some(ref_id.as_str()),
        "request_secret returns the same ref it published: {requested}"
    );
    assert_eq!(
        requested.get("key_name").and_then(Value::as_str),
        Some("PLANTED_API_KEY")
    );
    assert!(
        !requested.to_string().contains("planted-secret-value"),
        "the raw secret must NEVER cross the model-facing surface: {requested}"
    );

    // A second submit against the same ref is refused — the ref is single-use.
    let replay = rpc(
        &harness.rpc_base,
        212,
        "openhuman.mcp_setup_submit_secret",
        json!({ "ref_id": ref_id, "value": "a-different-value" }),
    )
    .await;
    let message = error_message(&replay, "mcp_setup_submit_secret replay");
    assert!(
        message.contains("unknown or already submitted"),
        "replaying a ref must be refused, got: {message}"
    );

    // A syntactically invalid ref never reaches the vault.
    let malformed = rpc(
        &harness.rpc_base,
        213,
        "openhuman.mcp_setup_submit_secret",
        json!({ "ref_id": "../../etc/passwd", "value": "nope" }),
    )
    .await;
    assert!(
        error_message(&malformed, "mcp_setup_submit_secret malformed").contains("invalid ref_id"),
        "a malformed ref must be refused by the parser: {malformed}"
    );

    // And the request half validates its own inputs before touching the vault.
    let blank_key = rpc(
        &harness.rpc_base,
        214,
        "openhuman.mcp_setup_request_secret",
        json!({ "key_name": "  ", "prompt": "anything" }),
    )
    .await;
    assert!(
        error_message(&blank_key, "request_secret blank key")
            .contains("key_name must not be empty"),
        "a blank key_name must be refused without blocking for five minutes: {blank_key}"
    );

    harness.join.abort();
}

// ── mcp_setup: the install half ─────────────────────────────────────────────

/// `mcp_setup_test_connection` and `mcp_setup_install_and_connect`.
///
/// Both take `{ENV_KEY: secret://…}` handles, and both parse those handles
/// before doing anything else — which is the boundary this case asserts, since
/// spawning a real candidate server would need a registry-resolvable package
/// and a subprocess.
///
/// `test_connection` is additionally documented to report a failed dial as
/// `ok: false` **with the reason**, rather than raising — that contract is what
/// lets the setup agent retry instead of aborting, and it is asserted here
/// against a name the fixture registry does not publish.
#[tokio::test]
async fn mcp_setup_install_paths_validate_handles_and_report_dial_failure_in_band() {
    let _lock = env_lock();
    let (registry_addr, registry_join) = serve_fixture_registry().await;
    let harness = setup(vec![EnvVarGuard::set(
        "MCP_OFFICIAL_REGISTRY_BASE",
        &format!("http://{registry_addr}"),
    )])
    .await;

    // A handle that is not a `secret://<hex>` ref is rejected by the parser,
    // for both methods, before any dial.
    for (id, method) in [
        (220, "openhuman.mcp_setup_test_connection"),
        (221, "openhuman.mcp_setup_install_and_connect"),
    ] {
        let bad_handle = rpc(
            &harness.rpc_base,
            id,
            method,
            json!({
                "qualified_name": INSTALLABLE,
                "env_refs": { "PLANTED_API_KEY": "not-a-ref!!" }
            }),
        )
        .await;
        assert!(
            error_message(&bad_handle, method).contains("invalid ref_id"),
            "{method} must refuse a malformed handle: {bad_handle}"
        );
    }

    // A blank qualified_name is refused ahead of the handles.
    let blank = rpc(
        &harness.rpc_base,
        222,
        "openhuman.mcp_setup_test_connection",
        json!({ "qualified_name": "", "env_refs": {} }),
    )
    .await;
    assert!(
        error_message(&blank, "test_connection blank name")
            .contains("qualified_name must not be empty"),
        "a blank name must be refused: {blank}"
    );

    // `env_refs` is required, not defaulted — omitting it must not silently
    // dial with no credentials.
    let no_refs = rpc(
        &harness.rpc_base,
        223,
        "openhuman.mcp_setup_install_and_connect",
        json!({ "qualified_name": INSTALLABLE }),
    )
    .await;
    assert!(
        error_message(&no_refs, "install_and_connect no env_refs").contains("env_refs"),
        "an absent env_refs must be named in the refusal: {no_refs}"
    );

    // The in-band failure contract: a dial that cannot happen is reported as
    // `ok: false` with a reason, NOT as a JSON-RPC error.
    let dial = rpc(
        &harness.rpc_base,
        224,
        "openhuman.mcp_setup_test_connection",
        json!({ "qualified_name": UNINSTALLABLE, "env_refs": {} }),
    )
    .await;
    let dial = ok(
        &dial,
        "mcp_setup_test_connection must not raise on a failed dial",
    );
    assert_eq!(
        dial.get("ok").and_then(Value::as_bool),
        Some(false),
        "a dial that could not succeed must report ok:false: {dial}"
    );
    assert!(
        dial.get("error")
            .and_then(Value::as_str)
            .is_some_and(|e| !e.trim().is_empty()),
        "ok:false must carry a non-empty reason the agent can act on: {dial}"
    );
    assert!(
        dial.get("tools").is_none_or(Value::is_null),
        "`tools` must be absent when the dial failed: {dial}"
    );

    // `install_and_connect` still raises here, and after #6110 that is the
    // *correct* answer rather than the drift this case originally pinned.
    //
    // `UNINSTALLABLE` declares neither a remote nor a package and the fixture
    // registry serves no version of it, so the **install** step is what fails.
    // There is no server and no `server_id`, so there is nothing to report a
    // `status` for — the schema's `installed_disconnected` means "install
    // succeeded, connect failed", which is a different case and not reachable
    // from this input.
    //
    // The reconciliation the old note asked for has happened: a failed
    // *connect* now returns `Ok` with `status: "installed_disconnected"` and an
    // `error`, matching `test_connection`'s in-band contract. Exercising that
    // arm needs a package that installs and then refuses to dial, which means a
    // registry-resolvable package and a subprocess; it is covered by the unit
    // tests on `classify_install_connect` in
    // `src/openhuman/mcp/registry/setup_ops_tests.rs`.
    let install = rpc(
        &harness.rpc_base,
        225,
        "openhuman.mcp_setup_install_and_connect",
        json!({ "qualified_name": UNINSTALLABLE, "env_refs": {} }),
    )
    .await;
    assert!(
        install.get("error").is_some(),
        "an install that cannot resolve a package must still raise — it has no \
         server_id to attach a status to: {install}"
    );

    registry_join.abort();
    harness.join.abort();
}

// ── mcp_clients ─────────────────────────────────────────────────────────────

/// The four uncovered `mcp_clients_*` controllers.
///
/// `registry_get` reads the fixture catalog; `detect_auth` and `oauth_begin`
/// address an *installed* server by UUID, so with nothing installed the
/// interesting assertion is that they refuse a name they cannot resolve rather
/// than dialling something arbitrary. `config_assist` needs both the catalog
/// and an inference turn, so it is asserted at its catalog boundary.
#[tokio::test]
async fn mcp_clients_registry_get_and_the_per_server_controllers() {
    let _lock = env_lock();
    let (registry_addr, registry_join) = serve_fixture_registry().await;
    let harness = setup(vec![EnvVarGuard::set(
        "MCP_OFFICIAL_REGISTRY_BASE",
        &format!("http://{registry_addr}"),
    )])
    .await;

    // `mcp_clients_registry_get` — the install dialog's one round trip, which
    // must carry the credential names alongside the detail.
    let detail = rpc(
        &harness.rpc_base,
        230,
        "openhuman.mcp_clients_registry_get",
        json!({ "qualified_name": INSTALLABLE }),
    )
    .await;
    let detail = ok(&detail, "mcp_clients_registry_get");
    let server = detail
        .get("server")
        .unwrap_or_else(|| panic!("no `server` in registry_get: {detail}"));
    assert_eq!(
        server.get("qualified_name").and_then(Value::as_str),
        Some(INSTALLABLE)
    );
    assert_eq!(
        server
            .get("required_env_keys")
            .and_then(Value::as_array)
            .map(|k| k.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
        Some(vec!["PLANTED_API_KEY"]),
        "registry_get must inject the credential names, not make the dialog \
         fetch them separately: {server}"
    );
    assert_eq!(
        server.get("source").and_then(Value::as_str),
        Some("mcp_official"),
        "the detail must name the registry it came from so an install can route \
         its lookup back: {server}"
    );

    let blank = rpc(
        &harness.rpc_base,
        231,
        "openhuman.mcp_clients_registry_get",
        json!({ "qualified_name": " " }),
    )
    .await;
    assert!(
        error_message(&blank, "registry_get blank").contains("qualified_name must not be empty"),
        "a blank name must be refused: {blank}"
    );

    // `detect_auth` / `oauth_begin` address an installed server by id. Nothing
    // is installed, so both must refuse — and refuse *by name*, so an operator
    // can tell "no such server" from "the probe failed".
    for (id, method) in [
        (232, "openhuman.mcp_clients_detect_auth"),
        (233, "openhuman.mcp_clients_oauth_begin"),
    ] {
        let blank_id = rpc(&harness.rpc_base, id, method, json!({ "server_id": "  " })).await;
        assert!(
            error_message(&blank_id, method).contains("server_id must not be empty"),
            "{method} must refuse a blank server_id: {blank_id}"
        );

        let unknown = rpc(
            &harness.rpc_base,
            id + 100,
            method,
            json!({ "server_id": "00000000-0000-4000-8000-000000000000" }),
        )
        .await;
        assert!(
            unknown.get("error").is_some(),
            "{method} against an uninstalled server id must error rather than \
             probing something arbitrary: {unknown}"
        );
        // A UUID that resolves to nothing must not come back as a usable
        // authorize URL — that would be an open redirect waiting to happen.
        assert!(
            unknown.get("result").is_none(),
            "{method} must not return a result for an unknown server: {unknown}"
        );
    }

    // `config_assist` validates its identifier, then needs the catalog detail
    // before it can build a prompt. A name the registry cannot resolve fails at
    // that fetch, with the message saying so.
    let assist_blank = rpc(
        &harness.rpc_base,
        240,
        "openhuman.mcp_clients_config_assist",
        json!({ "qualified_name": "", "user_message": "how do I set this up?" }),
    )
    .await;
    assert!(
        error_message(&assist_blank, "config_assist blank")
            .contains("qualified_name must not be empty"),
        "config_assist must refuse a blank name: {assist_blank}"
    );

    let assist_unknown = rpc(
        &harness.rpc_base,
        241,
        "openhuman.mcp_clients_config_assist",
        json!({ "qualified_name": UNINSTALLABLE, "user_message": "how do I set this up?" }),
    )
    .await;
    let message = error_message(&assist_unknown, "config_assist unknown server");
    assert!(
        message.contains("Failed to fetch registry detail"),
        "config_assist must say the catalog lookup is what failed, not surface a \
         bare transport error, got: {message}"
    );

    // `user_message` is required — a caller that forgets it must be told, not
    // handed an assistant turn on an empty question.
    let assist_no_message = rpc(
        &harness.rpc_base,
        242,
        "openhuman.mcp_clients_config_assist",
        json!({ "qualified_name": INSTALLABLE }),
    )
    .await;
    assert!(
        error_message(&assist_no_message, "config_assist no message").contains("user_message"),
        "an absent user_message must be named: {assist_no_message}"
    );

    registry_join.abort();
    harness.join.abort();
}

// ── mcp_audit ───────────────────────────────────────────────────────────────

/// `mcp_audit_list` — the write-audit log's only reader, exercised over rows
/// this case records through the audit API so the content is knowable.
#[tokio::test]
async fn mcp_audit_list_filters_and_orders_the_write_log() {
    let _lock = env_lock();
    let harness = setup(Vec::new()).await;

    // Empty first: an audit log with no rows must read as an empty list, not
    // as an error — an operator opening the panel on a fresh install sees this.
    let empty = rpc(
        &harness.rpc_base,
        250,
        "openhuman.mcp_audit_list",
        json!({}),
    )
    .await;
    assert_eq!(
        ok(&empty, "mcp_audit_list empty")
            .get("records")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(0),
        "a fresh workspace must list zero records: {empty}"
    );

    // Seed three rows: two clients, two tools, one failure.
    let config = openhuman_core::openhuman::config::Config {
        workspace_dir: harness.workspace.clone(),
        ..openhuman_core::openhuman::config::Config::default()
    };
    use openhuman_core::openhuman::mcp::audit::{record_write, NewMcpWriteRecord};
    for (ts, client, tool, success, error) in [
        (1_000_i64, "mcp:claude-desktop", "memory.store", true, None),
        (
            2_000,
            "mcp:claude-desktop",
            "memory.delete",
            false,
            Some("denied by policy"),
        ),
        (3_000, "mcp:some-other-client", "memory.store", true, None),
    ] {
        record_write(
            &config,
            NewMcpWriteRecord {
                timestamp_ms: ts,
                client_info: client.to_string(),
                tool_name: tool.to_string(),
                args_summary: json!({ "keys": ["a", "b"] }),
                resulting_chunk_id: None,
                success,
                error_message: error.map(str::to_string),
            },
        )
        .expect("record an audit row");
    }

    let all = rpc(
        &harness.rpc_base,
        251,
        "openhuman.mcp_audit_list",
        json!({}),
    )
    .await;
    let all = ok(&all, "mcp_audit_list all");
    let records = all
        .get("records")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("no `records` array: {all}"));
    assert_eq!(records.len(), 3, "all three rows must come back: {all}");
    assert_eq!(
        records
            .iter()
            .filter_map(|r| r.get("timestamp_ms").and_then(Value::as_i64))
            .collect::<Vec<_>>(),
        vec![3_000, 2_000, 1_000],
        "records must be ordered newest-first, as the schema documents: {all}"
    );
    // The failed attempt is retained, with its reason — an audit log that kept
    // only successes would be useless for the thing it exists for.
    let failed = records
        .iter()
        .find(|r| r.get("success").and_then(Value::as_bool) == Some(false))
        .unwrap_or_else(|| panic!("the rejected write must be retained: {all}"));
    assert_eq!(
        failed.get("error_message").and_then(Value::as_str),
        Some("denied by policy"),
        "a failed write must keep its reason: {failed}"
    );
    assert_eq!(
        failed.get("tool_name").and_then(Value::as_str),
        Some("memory.delete")
    );

    // Each filter narrows, and they are exact matches rather than substrings.
    let by_client = rpc(
        &harness.rpc_base,
        252,
        "openhuman.mcp_audit_list",
        json!({ "client_filter": "mcp:some-other-client" }),
    )
    .await;
    let by_client = ok(&by_client, "mcp_audit_list client_filter");
    let rows = by_client
        .get("records")
        .and_then(Value::as_array)
        .expect("records");
    assert_eq!(
        rows.len(),
        1,
        "the client filter must narrow to one: {by_client}"
    );
    assert_eq!(
        rows[0].get("client_info").and_then(Value::as_str),
        Some("mcp:some-other-client")
    );

    let by_tool = rpc(
        &harness.rpc_base,
        253,
        "openhuman.mcp_audit_list",
        json!({ "tool_filter": "memory.store" }),
    )
    .await;
    assert_eq!(
        ok(&by_tool, "mcp_audit_list tool_filter")
            .get("records")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(2),
        "the tool filter must match exactly two: {by_tool}"
    );

    // `memory.` is a prefix of both tool names — an exact filter must match
    // neither, which is what stops a filter quietly widening.
    let prefix = rpc(
        &harness.rpc_base,
        254,
        "openhuman.mcp_audit_list",
        json!({ "tool_filter": "memory." }),
    )
    .await;
    assert_eq!(
        ok(&prefix, "mcp_audit_list prefix filter")
            .get("records")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(0),
        "the tool filter is exact, not a prefix match: {prefix}"
    );

    let successes = rpc(
        &harness.rpc_base,
        255,
        "openhuman.mcp_audit_list",
        json!({ "success_only": true }),
    )
    .await;
    let successes = ok(&successes, "mcp_audit_list success_only");
    let rows = successes
        .get("records")
        .and_then(Value::as_array)
        .expect("records");
    assert_eq!(
        rows.len(),
        2,
        "success_only must drop the failure: {successes}"
    );
    assert!(
        rows.iter()
            .all(|r| r.get("success").and_then(Value::as_bool) == Some(true)),
        "success_only must return only successes: {successes}"
    );

    let since = rpc(
        &harness.rpc_base,
        256,
        "openhuman.mcp_audit_list",
        json!({ "since_ms": 2_000 }),
    )
    .await;
    assert_eq!(
        ok(&since, "mcp_audit_list since_ms")
            .get("records")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(2),
        "since_ms is inclusive of its own boundary: {since}"
    );

    // Paging: limit then offset walk the newest-first set without overlap.
    let page_one = rpc(
        &harness.rpc_base,
        257,
        "openhuman.mcp_audit_list",
        json!({ "limit": 1 }),
    )
    .await;
    let page_two = rpc(
        &harness.rpc_base,
        258,
        "openhuman.mcp_audit_list",
        json!({ "limit": 1, "offset": 1 }),
    )
    .await;
    let first = ok(&page_one, "audit page one")
        .get("records")
        .and_then(Value::as_array)
        .expect("records")
        .clone();
    let second = ok(&page_two, "audit page two")
        .get("records")
        .and_then(Value::as_array)
        .expect("records")
        .clone();
    assert_eq!(first.len(), 1, "limit 1 returns one row: {page_one}");
    assert_eq!(
        second.len(),
        1,
        "limit 1 offset 1 returns one row: {page_two}"
    );
    assert_eq!(
        first[0].get("timestamp_ms").and_then(Value::as_i64),
        Some(3_000)
    );
    assert_eq!(
        second[0].get("timestamp_ms").and_then(Value::as_i64),
        Some(2_000),
        "offset must skip from the newest end, not restart: {page_two}"
    );

    harness.join.abort();
}
