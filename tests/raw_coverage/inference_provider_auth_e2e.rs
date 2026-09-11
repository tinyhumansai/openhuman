//! JSON-RPC E2E coverage for the seven `inference` controllers that no
//! `tests/**/*_e2e.rs` target reached: the four Claude-Code-CLI provider
//! surfaces, the Codex-CLI OAuth import, the BYO-provider auth-error registry,
//! and model-hint resolution.
//!
//! This file is a **module** of the aggregated `raw_coverage_all` target;
//! `build.rs` globs `tests/raw_coverage/` and generates the `mod` list.
//!
//! Run with:
//! `cargo test --test raw_coverage_all --features "$(bash scripts/ci/product-features.sh)"`
//!
//! # How each case stays deterministic without a network or a real CLI
//!
//! - `claude_code_status` shells out to a `claude` binary. Every case here
//!   points `OPENHUMAN_CLAUDE_CLI` at a **stub script** whose version string
//!   the case chooses, so `ok` / `outdated` / `unusable` / `not_installed` are
//!   all reachable and none of them depend on what is installed on the host.
//! - `claude_code_auth_status` resolves `ANTHROPIC_API_KEY` *before* it spawns
//!   anything, so the `api_key_env` branch is assertable with no subprocess at
//!   all. The un-keyed branch is driven through the same stub.
//! - `openai_oauth_import_codex_cli` reads `$CODEX_HOME/auth.json`, so a
//!   fixture file drives both the import and the failure.
//! - `provider_auth_errors` reads a process-lived registry that
//!   `auth_error_registry::record` writes; the case records and clears its own
//!   entries so it leaves no state behind for another suite.
//!
//! Env is process-global and every aggregated suite shares one process, so
//! each case takes the **crate-wide** [`env_lock`] for its whole body.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::http::header::AUTHORIZATION;
use reqwest::StatusCode;
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

use openhuman_core::core::auth::{get_rpc_token, init_rpc_token, CORE_TOKEN_ENV_VAR};
use openhuman_core::core::jsonrpc::build_core_http_router;
use openhuman_core::openhuman::inference::auth_error_registry;

const TEST_RPC_TOKEN: &str = "inference-provider-auth-e2e-token";

static AUTH_INIT: OnceLock<()> = OnceLock::new();

/// Crate-wide, not file-local: all aggregated suites share one process, so a
/// private mutex would not mutually exclude with anyone else's env mutation.
static ENV_LOCK: &OnceLock<Mutex<()>> = &crate::SHARED_ENV_LOCK;

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct EnvVarGuard {
    key: &'static str,
    old: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set_to_path(key: &'static str, path: &Path) -> Self {
        let old = std::env::var_os(key);
        std::env::set_var(key, path.as_os_str());
        Self { key, old }
    }

    fn set(key: &'static str, value: &str) -> Self {
        let old = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, old }
    }

    fn unset(key: &'static str) -> Self {
        let old = std::env::var_os(key);
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

/// See the note in `sandbox_runtime_platform_e2e.rs`: `core::auth::RPC_TOKEN`
/// is a process-global `OnceLock` and `init_rpc_token` is idempotent, so inside
/// the aggregated binary the first suite to initialise pins the bearer. Use the
/// token this process actually validates rather than assuming ours won.
fn ensure_rpc_auth() -> String {
    AUTH_INIT.get_or_init(|| {
        if std::env::var(CORE_TOKEN_ENV_VAR)
            .ok()
            .filter(|v| !v.trim().is_empty())
            .is_none()
        {
            std::env::set_var(CORE_TOKEN_ENV_VAR, TEST_RPC_TOKEN);
        }
        let token_dir = std::env::temp_dir().join("openhuman-inference-provider-auth-e2e-auth");
        init_rpc_token(&token_dir).expect("init rpc auth token");
    });
    get_rpc_token()
        .expect("core RPC token must be initialised before serving")
        .to_string()
}

fn write_config(openhuman_dir: &Path, extra: &str) {
    std::fs::create_dir_all(openhuman_dir).expect("create .openhuman");
    let cfg = format!(
        r#"api_url = "http://127.0.0.1:9"
default_model = "e2e-model"
default_temperature = 0.2
{extra}
[secrets]
encrypt = false

[local_ai]
enabled = false

[modules]
enabled = false
allow_download = false

[memory]
provider = "none"
embedding_provider = "none"
embedding_model = "none"
embedding_dimensions = 0

[memory_tree]
embedding_strict = false
"#
    );
    std::fs::write(openhuman_dir.join("config.toml"), &cfg).expect("write config.toml");
    let _: openhuman_core::openhuman::config::Config =
        toml::from_str(&cfg).expect("test config must match schema");
}

struct TestHarness {
    tmp: TempDir,
    _guards: Vec<EnvVarGuard>,
    rpc_base: String,
    token: String,
    join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
}

impl TestHarness {
    fn home(&self) -> &Path {
        self.tmp.path()
    }

    fn workspace(&self) -> std::path::PathBuf {
        self.tmp.path().join("workspace")
    }

    async fn rpc(&self, id: i64, method: &str, params: Value) -> Value {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("client");
        let url = format!("{}/rpc", self.rpc_base.trim_end_matches('/'));
        let response = client
            .post(&url)
            .header(AUTHORIZATION, format!("Bearer {}", self.token))
            .json(&json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params,
            }))
            .send()
            .await
            .unwrap_or_else(|err| panic!("POST {url} {method}: {err}"));
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "HTTP transport should accept {method}"
        );
        response
            .json::<Value>()
            .await
            .unwrap_or_else(|err| panic!("json for {method}: {err}"))
    }
}

async fn serve_rpc() -> (
    SocketAddr,
    String,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    let token = ensure_rpc_auth();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind rpc listener");
    let addr = listener.local_addr().expect("rpc listener addr");
    let router = build_core_http_router(false);
    let join = tokio::spawn(async move { axum::serve(listener, router).await });
    (addr, token, join)
}

/// `extra` is appended to the generated `config.toml`, which is how a case
/// pins a per-workload provider route for `inference_resolve_model`.
async fn setup(extra: &str) -> TestHarness {
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    write_config(&home.join(".openhuman"), extra);
    let workspace = home.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");

    let guards = vec![
        EnvVarGuard::set_to_path("HOME", home),
        EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace),
        EnvVarGuard::unset("BACKEND_URL"),
        EnvVarGuard::unset("VITE_BACKEND_URL"),
        EnvVarGuard::unset("OPENHUMAN_API_URL"),
        // The host developer's real credentials must never decide what these
        // cases observe.
        EnvVarGuard::unset("ANTHROPIC_API_KEY"),
        EnvVarGuard::unset("OPENHUMAN_CLAUDE_CLI"),
        EnvVarGuard::unset("CODEX_HOME"),
        EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_STRICT", "false"),
    ];

    let (addr, token, join) = serve_rpc().await;
    TestHarness {
        tmp,
        _guards: guards,
        rpc_base: format!("http://{addr}"),
        token,
        join,
    }
}

fn ok<'a>(value: &'a Value, context: &str) -> &'a Value {
    if let Some(error) = value.get("error") {
        panic!("{context}: unexpected JSON-RPC error: {error}");
    }
    value
        .get("result")
        .unwrap_or_else(|| panic!("{context}: missing result: {value}"))
}

fn err_message(value: &Value, context: &str) -> String {
    let error = value
        .get("error")
        .unwrap_or_else(|| panic!("{context}: expected JSON-RPC error, got: {value}"));
    error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{context}: error without message: {error}"))
        .to_string()
}

fn payload<'a>(value: &'a Value, context: &str) -> &'a Value {
    let result = ok(value, context);
    result.get("result").unwrap_or(result)
}

/// Write an executable stub `claude` and return its path. `body` is a POSIX
/// shell script; the caller decides what `--version` prints and what the exit
/// status is.
#[cfg(unix)]
fn write_claude_stub(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir).expect("create stub dir");
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write claude stub");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod stub");
    path
}

/// A JWT-shaped token with an unsigned, base64url payload — what the Codex CLI
/// writes and what the importer decodes for the account id and expiry.
fn unsigned_jwt(claims: Value) -> String {
    use base64::Engine as _;
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let header = engine.encode(br#"{"alg":"none","typ":"JWT"}"#);
    let payload = engine.encode(claims.to_string().as_bytes());
    format!("{header}.{payload}.")
}

// ── inference.resolve_model ────────────────────────────────────────────────

#[tokio::test]
async fn inference_resolve_model_maps_hints_and_tiers_to_the_routed_model() {
    let _lock = env_lock();
    let harness = setup("").await;

    // ---- Phase A: nothing routed. Every hint resolves to its managed tier. --

    // A managed tier with no BYOK route resolves to the tier name itself —
    // the managed backend is what expands it.
    let reasoning = harness
        .rpc(
            71_001,
            "openhuman.inference_resolve_model",
            json!({ "hint": "hint:reasoning" }),
        )
        .await;
    let reasoning = payload(&reasoning, "resolve_model hint:reasoning");
    assert_eq!(
        reasoning.get("model"),
        Some(&json!("reasoning-v1")),
        "an unrouted reasoning hint resolves to the managed tier: {reasoning}"
    );
    assert_eq!(
        reasoning.get("vision"),
        Some(&json!(true)),
        "the reasoning tier is one of the two vision-capable managed tiers \
         (`oh_tier_supports_vision`); the RPC schema comment claiming the \
         per-tier map is `currently all false` is stale — see \
         ~/tinyhuman/bugs/e2e-wave-inference-stale-vision-and-workspace-docs.md: \
         {reasoning}"
    );

    // The bare tier name is accepted alongside the `hint:` alias and must
    // resolve identically.
    let bare_tier = harness
        .rpc(
            71_002,
            "openhuman.inference_resolve_model",
            json!({ "hint": "reasoning-v1" }),
        )
        .await;
    assert_eq!(
        payload(&bare_tier, "resolve_model reasoning-v1").get("model"),
        Some(&json!("reasoning-v1"))
    );

    // A tier that is genuinely text-only answers `vision: false`, so the flag
    // carries real information rather than being pinned one way.
    let chat = harness
        .rpc(
            71_003,
            "openhuman.inference_resolve_model",
            json!({ "hint": "hint:chat" }),
        )
        .await;
    let chat = payload(&chat, "resolve_model hint:chat");
    assert_eq!(chat.get("model"), Some(&json!("chat-v1")));
    assert_eq!(
        chat.get("vision"),
        Some(&json!(false)),
        "the chat tier is text-only: {chat}"
    );

    for (id, hint, tier) in [
        (71_004, "hint:coding", "coding-v1"),
        (71_005, "hint:agentic", "agentic-v1"),
        (71_006, "hint:burst", "burst-v1"),
    ] {
        let resolved = harness
            .rpc(
                id,
                "openhuman.inference_resolve_model",
                json!({ "hint": hint }),
            )
            .await;
        assert_eq!(
            payload(&resolved, hint).get("model"),
            Some(&json!(tier)),
            "{hint} must resolve to {tier} while nothing is routed"
        );
    }

    // An unknown hint is passed through rather than silently remapped.
    let unknown = harness
        .rpc(
            71_007,
            "openhuman.inference_resolve_model",
            json!({ "hint": "hint:not-a-workload" }),
        )
        .await;
    assert_eq!(
        payload(&unknown, "resolve_model unknown hint").get("model"),
        Some(&json!("hint:not-a-workload")),
        "an unrecognised hint must round-trip, not resolve to a wrong model"
    );

    // ---- Phase B: pin ONE BYOK route and watch who inherits it. -------------
    //
    // Set through the RPC the settings panel uses, not by hand-writing a key
    // into `config.toml`, so this is a two-controller round trip: what the
    // settings call persisted vs what the router then resolves.
    let routed = harness
        .rpc(
            71_010,
            "openhuman.inference_update_model_settings",
            json!({ "coding_provider": "openrouter:zai/glm-4.7" }),
        )
        .await;
    assert!(
        routed.get("error").is_none(),
        "pinning the coding route must succeed: {routed}"
    );

    // The routed role resolves to the model half only — the provider slug is
    // routing, not a model id.
    for (id, hint) in [(71_011, "hint:coding"), (71_012, "coding-v1")] {
        let coding = harness
            .rpc(
                id,
                "openhuman.inference_resolve_model",
                json!({ "hint": hint }),
            )
            .await;
        assert_eq!(
            payload(&coding, hint).get("model"),
            Some(&json!("zai/glm-4.7")),
            "{hint}: a `slug:model` route must resolve to the model half only"
        );
    }

    // The part that is easy to get wrong, and the reason this is two phases.
    //
    // This block used to assert the opposite: `provider_for_role` let the three
    // chat-tier roles inherit any configured BYOK route from a sibling, so
    // pinning *only* `coding` also moved chat and reasoning off the managed
    // backend — the user's ordinary conversations silently billed to their own
    // key, with no setting saying so. #6109 removed that inheritance; each route
    // now stands alone. The assertion is inverted rather than deleted, because
    // "setting one route does not move the others" is precisely the property
    // that needs a guard.
    for (id, hint, tier) in [
        (71_013, "hint:reasoning", "reasoning-v1"),
        (71_014, "hint:chat", "chat-v1"),
    ] {
        let sibling = harness
            .rpc(
                id,
                "openhuman.inference_resolve_model",
                json!({ "hint": hint }),
            )
            .await;
        assert_eq!(
            payload(&sibling, hint).get("model"),
            Some(&json!(tier)),
            "{hint} was never configured, so it must stay on the managed backend \
             rather than inherit the BYOK route pinned for `coding` (#6109)"
        );
    }

    // Agentic, burst, vision and the background workloads stay on the managed
    // backend for the same reason, and always did — they run tier-specific
    // models a BYOK provider does not serve.
    for (id, hint, tier) in [
        (71_015, "hint:agentic", "agentic-v1"),
        (71_016, "hint:burst", "burst-v1"),
        (71_017, "hint:vision", "vision-v1"),
        (71_018, "hint:summarization", "summarization-v1"),
    ] {
        let managed = harness
            .rpc(
                id,
                "openhuman.inference_resolve_model",
                json!({ "hint": hint }),
            )
            .await;
        assert_eq!(
            payload(&managed, hint).get("model"),
            Some(&json!(tier)),
            "{hint} must NOT inherit a chat-tier BYOK route — it stays managed"
        );
    }

    // ---- Failure paths -----------------------------------------------------
    //
    // Omitted: refused at the dispatch boundary from the schema's `required`
    // flag. Present-but-wrong-type: refused by the handler's deserialize. Two
    // different gates, so assert the two different messages.
    let missing = harness
        .rpc(71_020, "openhuman.inference_resolve_model", json!({}))
        .await;
    assert!(
        err_message(&missing, "resolve_model missing hint")
            .contains("missing required param 'hint'"),
        "an omitted `hint` must be refused by the schema gate"
    );

    // Present but the wrong JSON type. This is a THIRD gate, distinct from the
    // two above: `core::all::validate_params` type-checks every present param
    // against its declared `TypeSchema` before dispatch, so the handler's own
    // `serde_json::from_value` is never reached. Pinning the exact wording
    // because it is the message a client developer sees.
    let wrong_type = harness
        .rpc(
            71_021,
            "openhuman.inference_resolve_model",
            json!({ "hint": 42 }),
        )
        .await;
    assert_eq!(
        err_message(&wrong_type, "resolve_model numeric hint"),
        "invalid type for param 'hint' in inference.resolve_model: \
         expected string, got number",
        "a non-string `hint` must be refused by the boundary type-check"
    );

    harness.join.abort();
}

// ── inference.provider_auth_errors ─────────────────────────────────────────

#[tokio::test]
async fn inference_provider_auth_errors_surfaces_recorded_byo_key_rejections() {
    let _lock = env_lock();
    let harness = setup("").await;

    // Start from a known state — another suite in this binary may have
    // recorded entries, and this case owns only its own slugs.
    let _ = auth_error_registry::clear("e2e-openrouter");
    let _ = auth_error_registry::clear("e2e-deepseek");

    let empty = harness
        .rpc(
            72_001,
            "openhuman.inference_provider_auth_errors",
            json!({}),
        )
        .await;
    let before = payload(&empty, "provider_auth_errors empty")
        .get("errors")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("errors must be an array: {empty}"))
        .clone();
    assert!(
        !before
            .iter()
            .any(|e| e.get("provider") == Some(&json!("e2e-openrouter"))),
        "this case's slug must not be present before it records: {before:?}"
    );

    // Record two failures the way the demote site does, then read them back
    // over the RPC. This is the whole contract of the namespace: what the
    // provider layer records is what the settings panel shows.
    assert!(
        auth_error_registry::record("e2e-openrouter", 401),
        "the first failure for a provider opens a new episode"
    );
    assert!(
        !auth_error_registry::record("e2e-openrouter", 401),
        "a repeat failure must refresh, not re-open — that latch is what keeps \
         a retry loop from re-flooding the notification centre"
    );
    auth_error_registry::record("e2e-deepseek", 403);

    let listed = harness
        .rpc(
            72_002,
            "openhuman.inference_provider_auth_errors",
            json!({}),
        )
        .await;
    let errors = payload(&listed, "provider_auth_errors recorded")
        .get("errors")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("errors must be an array: {listed}"))
        .clone();

    let openrouter = errors
        .iter()
        .find(|e| e.get("provider") == Some(&json!("e2e-openrouter")))
        .unwrap_or_else(|| panic!("the recorded 401 must be surfaced: {errors:?}"));
    assert_eq!(openrouter.get("status"), Some(&json!(401)));
    let message = openrouter
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        message.contains("e2e-openrouter") && message.contains("401"),
        "the surfaced message must name the provider and the status: {message}"
    );
    assert!(
        message.contains("Update your"),
        "the message must be actionable — it is rendered verbatim in the UI: {message}"
    );
    assert!(
        openrouter
            .get("timestamp_ms")
            .and_then(Value::as_u64)
            .is_some_and(|t| t > 0),
        "each entry must carry a wall-clock timestamp: {openrouter}"
    );

    let deepseek = errors
        .iter()
        .find(|e| e.get("provider") == Some(&json!("e2e-deepseek")))
        .unwrap_or_else(|| panic!("the recorded 403 must be surfaced: {errors:?}"));
    assert_eq!(deepseek.get("status"), Some(&json!(403)));

    // Clearing a provider's key removes it from the notice — assert the read
    // side sees that, since a stale "your key is bad" banner after the user
    // fixed their key is the failure this registry exists to avoid.
    assert!(auth_error_registry::clear("e2e-openrouter"));
    let after = harness
        .rpc(
            72_003,
            "openhuman.inference_provider_auth_errors",
            json!({}),
        )
        .await;
    let after_errors = payload(&after, "provider_auth_errors cleared")
        .get("errors")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("errors must be an array: {after}"))
        .clone();
    assert!(
        !after_errors
            .iter()
            .any(|e| e.get("provider") == Some(&json!("e2e-openrouter"))),
        "a cleared provider must disappear from the notice: {after_errors:?}"
    );
    assert!(
        after_errors
            .iter()
            .any(|e| e.get("provider") == Some(&json!("e2e-deepseek"))),
        "clearing one provider must not clear the others: {after_errors:?}"
    );

    // Leave no state behind for another suite in this binary.
    let _ = auth_error_registry::clear("e2e-deepseek");

    harness.join.abort();
}

// ── inference.claude_code_status ───────────────────────────────────────────

#[cfg(unix)]
#[tokio::test]
async fn inference_claude_code_status_classifies_ok_outdated_unusable_and_missing() {
    let _lock = env_lock();
    let harness = setup("").await;
    let stub_dir = harness.home().join("stub-cli");

    // `ok` — a version at or above MIN_CLI_VERSION (2.0.0).
    let good = write_claude_stub(
        &stub_dir,
        "claude-good",
        "#!/bin/sh\necho '9.9.9 (Claude Code)'\n",
    );
    {
        let _cli = EnvVarGuard::set_to_path("OPENHUMAN_CLAUDE_CLI", &good);
        let status = harness
            .rpc(73_001, "openhuman.inference_claude_code_status", json!({}))
            .await;
        let status = payload(&status, "claude_code_status ok");
        assert_eq!(
            status.get("status"),
            Some(&json!("ok")),
            "a modern CLI must classify as ok: {status}"
        );
        assert_eq!(status.get("version"), Some(&json!("9.9.9")));
        assert_eq!(
            status.get("path").and_then(Value::as_str),
            Some(good.display().to_string().as_str()),
            "the resolved binary path must be reported: {status}"
        );
    }

    // `outdated` — below the minimum. The reply must carry the minimum so the
    // UI can tell the user what to upgrade to.
    let old = write_claude_stub(
        &stub_dir,
        "claude-old",
        "#!/bin/sh\necho '1.0.1 (Claude Code)'\n",
    );
    {
        let _cli = EnvVarGuard::set_to_path("OPENHUMAN_CLAUDE_CLI", &old);
        let status = harness
            .rpc(73_002, "openhuman.inference_claude_code_status", json!({}))
            .await;
        let status = payload(&status, "claude_code_status outdated");
        assert_eq!(status.get("status"), Some(&json!("outdated")));
        assert_eq!(status.get("version"), Some(&json!("1.0.1")));
        assert_eq!(
            status.get("min_required"),
            Some(&json!("2.0.0")),
            "an outdated verdict must name the required version: {status}"
        );
    }

    // `unusable` — the binary exists but fails. A non-zero exit must not be
    // reported as "not installed": telling a user to install what they already
    // have is the wrong instruction.
    let broken = write_claude_stub(
        &stub_dir,
        "claude-broken",
        "#!/bin/sh\necho 'boom' >&2\nexit 3\n",
    );
    {
        let _cli = EnvVarGuard::set_to_path("OPENHUMAN_CLAUDE_CLI", &broken);
        let status = harness
            .rpc(73_003, "openhuman.inference_claude_code_status", json!({}))
            .await;
        let status = payload(&status, "claude_code_status unusable");
        assert_eq!(status.get("status"), Some(&json!("unusable")));
        let reason = status
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(
            reason.contains("non-zero exit"),
            "an unusable verdict must explain itself: {status}"
        );
        assert!(
            reason.contains("boom"),
            "the CLI's own stderr must reach the reason: {status}"
        );
    }

    // `not_installed` — nothing on PATH and no override.
    let empty_path = harness.home().join("empty-path");
    std::fs::create_dir_all(&empty_path).expect("create empty path dir");
    {
        let _cli = EnvVarGuard::unset("OPENHUMAN_CLAUDE_CLI");
        let _path = EnvVarGuard::set_to_path("PATH", &empty_path);
        let status = harness
            .rpc(73_004, "openhuman.inference_claude_code_status", json!({}))
            .await;
        assert_eq!(
            payload(&status, "claude_code_status not installed").get("status"),
            Some(&json!("not_installed")),
            "no binary anywhere must classify as not_installed"
        );
    }

    // An override pointing at a path that does not exist falls back to the
    // PATH lookup rather than reporting `unusable` on a phantom binary.
    {
        let _cli = EnvVarGuard::set(
            "OPENHUMAN_CLAUDE_CLI",
            harness.home().join("nope/claude").to_str().unwrap(),
        );
        let _path = EnvVarGuard::set_to_path("PATH", &empty_path);
        let status = harness
            .rpc(73_005, "openhuman.inference_claude_code_status", json!({}))
            .await;
        assert_eq!(
            payload(&status, "claude_code_status bad override").get("status"),
            Some(&json!("not_installed")),
            "a non-existent override must not be treated as an installed binary"
        );
    }

    harness.join.abort();
}

// ── inference.claude_code_auth_status ──────────────────────────────────────

#[cfg(unix)]
#[tokio::test]
async fn inference_claude_code_auth_status_prefers_the_env_key_then_reads_the_cli() {
    let _lock = env_lock();
    let harness = setup("").await;
    let stub_dir = harness.home().join("stub-auth-cli");

    // `ANTHROPIC_API_KEY` wins outright — the spawned CLI would inherit it, so
    // it decides who authenticates regardless of any stored session. Assert it
    // short-circuits: the stub below would report a *subscription* if reached.
    let logged_in = write_claude_stub(
        &stub_dir,
        "claude-loggedin",
        "#!/bin/sh\necho '{\"loggedIn\":true,\"authMethod\":\"claude.ai\",\
         \"email\":\"e2e@example.test\",\"subscriptionType\":\"max\"}'\n",
    );
    {
        let _cli = EnvVarGuard::set_to_path("OPENHUMAN_CLAUDE_CLI", &logged_in);
        let _key = EnvVarGuard::set("ANTHROPIC_API_KEY", "sk-ant-e2e");
        let auth = harness
            .rpc(
                74_001,
                "openhuman.inference_claude_code_auth_status",
                json!({}),
            )
            .await;
        let auth = payload(&auth, "auth_status api key");
        assert_eq!(
            auth.get("source"),
            Some(&json!("api_key_env")),
            "an env API key must win over a logged-in CLI: {auth}"
        );
        assert!(
            auth.get("account_email").is_none(),
            "the api-key branch must not report a subscription account: {auth}"
        );
        assert!(
            auth.get("last_checked")
                .and_then(Value::as_u64)
                .is_some_and(|t| t > 0),
            "every probe must stamp when it ran: {auth}"
        );
    }

    // A blank key is not a key — the probe must fall through to the CLI.
    {
        let _cli = EnvVarGuard::set_to_path("OPENHUMAN_CLAUDE_CLI", &logged_in);
        let _key = EnvVarGuard::set("ANTHROPIC_API_KEY", "   ");
        let auth = harness
            .rpc(
                74_002,
                "openhuman.inference_claude_code_auth_status",
                json!({}),
            )
            .await;
        let auth = payload(&auth, "auth_status blank key");
        assert_eq!(
            auth.get("source"),
            Some(&json!("subscription")),
            "a whitespace-only key must not be mistaken for a credential: {auth}"
        );
        assert_eq!(
            auth.get("account_email"),
            Some(&json!("e2e@example.test")),
            "the subscription branch must carry the account email: {auth}"
        );
        assert_eq!(auth.get("subscription_type"), Some(&json!("max")));
    }

    // Signed out — the CLI says so explicitly, and only then may we say so.
    let logged_out = write_claude_stub(
        &stub_dir,
        "claude-loggedout",
        "#!/bin/sh\necho '{\"loggedIn\":false}'\n",
    );
    {
        let _cli = EnvVarGuard::set_to_path("OPENHUMAN_CLAUDE_CLI", &logged_out);
        let auth = harness
            .rpc(
                74_003,
                "openhuman.inference_claude_code_auth_status",
                json!({}),
            )
            .await;
        assert_eq!(
            payload(&auth, "auth_status signed out").get("source"),
            Some(&json!("none")),
            "an explicit loggedIn:false is the only thing that may read as signed out"
        );
    }

    // A CLI too old for `auth status` exits non-zero. That must be `unknown`,
    // never `none` — telling a signed-in user they are signed out because their
    // binary predates a subcommand is the documented anti-goal.
    let ancient = write_claude_stub(
        &stub_dir,
        "claude-ancient",
        "#!/bin/sh\necho 'unknown command' >&2\nexit 1\n",
    );
    {
        let _cli = EnvVarGuard::set_to_path("OPENHUMAN_CLAUDE_CLI", &ancient);
        let auth = harness
            .rpc(
                74_004,
                "openhuman.inference_claude_code_auth_status",
                json!({}),
            )
            .await;
        assert_eq!(
            payload(&auth, "auth_status ancient cli").get("source"),
            Some(&json!("unknown")),
            "a CLI that cannot answer must read as unknown, never as signed out"
        );
    }

    // Unparseable output is the same story.
    let garbage = write_claude_stub(
        &stub_dir,
        "claude-garbage",
        "#!/bin/sh\necho 'not json at all'\n",
    );
    {
        let _cli = EnvVarGuard::set_to_path("OPENHUMAN_CLAUDE_CLI", &garbage);
        let auth = harness
            .rpc(
                74_005,
                "openhuman.inference_claude_code_auth_status",
                json!({}),
            )
            .await;
        assert_eq!(
            payload(&auth, "auth_status garbage").get("source"),
            Some(&json!("unknown")),
            "unparseable CLI output must read as unknown, never as signed out"
        );
    }

    harness.join.abort();
}

// ── inference.claude_code_settings / set_full_access ───────────────────────

#[tokio::test]
async fn inference_claude_code_full_access_toggle_round_trips_through_the_workspace() {
    let _lock = env_lock();
    let harness = setup("").await;

    // The safe posture is the default, and it must be the default on a fresh
    // install with no settings file at all.
    let initial = harness
        .rpc(
            75_001,
            "openhuman.inference_claude_code_settings",
            json!({}),
        )
        .await;
    assert_eq!(
        payload(&initial, "claude_code_settings default").get("full_access"),
        Some(&json!(false)),
        "full access must default OFF — this gates bypassPermissions + Bash"
    );

    // Enable it, and confirm the change is persisted rather than merely echoed.
    let enabled = harness
        .rpc(
            75_002,
            "openhuman.inference_claude_code_set_full_access",
            json!({ "enabled": true }),
        )
        .await;
    assert_eq!(
        payload(&enabled, "set_full_access true").get("full_access"),
        Some(&json!(true))
    );
    // NOTE: despite the RPC description ("stored under the workspace") and the
    // helper's name (`workspace_dir_from_config`), the file lands in the
    // **config** directory — the function returns `config.config_path.parent()`,
    // i.e. `~/.openhuman`. Asserting the real location so this test documents
    // where the toggle actually is; the wording mismatch is filed in
    // ~/tinyhuman/bugs/e2e-wave-inference-stale-vision-and-workspace-docs.md.
    let settings_file = harness
        .home()
        .join(".openhuman")
        .join("claude_code_settings.json");
    assert!(
        settings_file.exists(),
        "the toggle must land on disk at {}",
        settings_file.display()
    );
    let on_disk: Value =
        serde_json::from_str(&std::fs::read_to_string(&settings_file).expect("read settings file"))
            .expect("settings file must be JSON");
    assert_eq!(
        on_disk.get("full_access"),
        Some(&json!(true)),
        "the persisted file must agree with the reply: {on_disk}"
    );

    let read_back = harness
        .rpc(
            75_003,
            "openhuman.inference_claude_code_settings",
            json!({}),
        )
        .await;
    assert_eq!(
        payload(&read_back, "claude_code_settings after enable").get("full_access"),
        Some(&json!(true))
    );

    // …and back off again.
    let disabled = harness
        .rpc(
            75_004,
            "openhuman.inference_claude_code_set_full_access",
            json!({ "enabled": false }),
        )
        .await;
    assert_eq!(
        payload(&disabled, "set_full_access false").get("full_access"),
        Some(&json!(false))
    );
    let read_back = harness
        .rpc(
            75_005,
            "openhuman.inference_claude_code_settings",
            json!({}),
        )
        .await;
    assert_eq!(
        payload(&read_back, "claude_code_settings after disable").get("full_access"),
        Some(&json!(false))
    );

    // A corrupt settings file must fail *safe* — defaults, not full access.
    std::fs::write(&settings_file, "{ this is not json").expect("corrupt the settings file");
    let corrupt = harness
        .rpc(
            75_006,
            "openhuman.inference_claude_code_settings",
            json!({}),
        )
        .await;
    assert_eq!(
        payload(&corrupt, "claude_code_settings corrupt").get("full_access"),
        Some(&json!(false)),
        "a corrupt settings file must never fail open into bypassPermissions"
    );

    // Failure path: `enabled` is required and must be a bool.
    let missing = harness
        .rpc(
            75_007,
            "openhuman.inference_claude_code_set_full_access",
            json!({}),
        )
        .await;
    assert!(
        err_message(&missing, "set_full_access missing")
            .contains("missing required param 'enabled'"),
        "an omitted `enabled` must be refused by the schema gate"
    );

    let wrong_type = harness
        .rpc(
            75_008,
            "openhuman.inference_claude_code_set_full_access",
            json!({ "enabled": "yes" }),
        )
        .await;
    // Same boundary type-check as `resolve_model` above — a string is not
    // coerced into a bool, which for *this* param would mean silently turning
    // on `bypassPermissions`.
    assert_eq!(
        err_message(&wrong_type, "set_full_access string"),
        "invalid type for param 'enabled' in inference.claude_code_set_full_access: \
         expected bool, got string",
        "a non-bool `enabled` must be rejected, not coerced to true"
    );

    harness.join.abort();
}

// ── inference.openai_oauth_import_codex_cli ────────────────────────────────

#[tokio::test]
async fn inference_openai_oauth_import_codex_cli_imports_a_real_auth_file() {
    let _lock = env_lock();
    let harness = setup("").await;

    let codex_home = harness.home().join("codex-home");
    std::fs::create_dir_all(&codex_home).expect("create codex home");
    let _codex = EnvVarGuard::set_to_path("CODEX_HOME", &codex_home);

    // No `auth.json` yet — the user never ran `codex login`. The error must be
    // the actionable one, because this is the common case the UI renders.
    let absent = harness
        .rpc(
            76_001,
            "openhuman.inference_openai_oauth_import_codex_cli",
            json!({}),
        )
        .await;
    let message = err_message(&absent, "import_codex_cli absent");
    assert!(
        message.contains("Could not read Codex CLI auth"),
        "a missing auth.json must say what it could not read: {message}"
    );
    assert!(
        message.contains("codex login"),
        "the error must tell the user the one command that fixes it: {message}"
    );

    // A file that is not JSON.
    std::fs::write(codex_home.join("auth.json"), "not json").expect("write bad auth.json");
    let unparseable = harness
        .rpc(
            76_002,
            "openhuman.inference_openai_oauth_import_codex_cli",
            json!({}),
        )
        .await;
    assert!(
        err_message(&unparseable, "import_codex_cli unparseable")
            .contains("Could not parse Codex CLI auth"),
        "a corrupt auth.json must be reported as a parse failure, not a missing file"
    );

    // Valid JSON with no tokens at all.
    std::fs::write(
        codex_home.join("auth.json"),
        json!({ "auth_mode": "chatgpt" }).to_string(),
    )
    .expect("write tokenless auth.json");
    let tokenless = harness
        .rpc(
            76_003,
            "openhuman.inference_openai_oauth_import_codex_cli",
            json!({}),
        )
        .await;
    assert!(
        err_message(&tokenless, "import_codex_cli tokenless").contains("has no tokens"),
        "a logged-out codex install must be distinguished from a corrupt one"
    );

    // The happy path: a real-shaped auth.json. The import must both report the
    // connection and leave a stored OAuth profile that `openai_oauth_status`
    // can see — an import that "succeeds" without persisting is the failure
    // this pair of assertions exists to catch.
    let access_token = unsigned_jwt(json!({
        "https://api.openai.com/auth": { "chatgpt_account_id": "acct_e2e" },
        "sub": "acct_subject",
        "exp": (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp(),
    }));
    std::fs::write(
        codex_home.join("auth.json"),
        json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "access_token": access_token,
                "refresh_token": "codex-refresh-e2e",
                "id_token": "codex-id-e2e",
            }
        })
        .to_string(),
    )
    .expect("write valid auth.json");

    let imported = harness
        .rpc(
            76_004,
            "openhuman.inference_openai_oauth_import_codex_cli",
            json!({}),
        )
        .await;
    let imported = payload(&imported, "import_codex_cli ok");
    assert_eq!(imported.get("connected"), Some(&json!(true)));
    assert_eq!(
        imported.get("provider"),
        Some(&json!("provider:openai")),
        "the credential-store provider key is the namespaced `provider:openai`, \
         not the bare slug: {imported}"
    );
    assert_eq!(imported.get("authMethod"), Some(&json!("oauth")));
    assert_eq!(
        imported.get("source"),
        Some(&json!("codex_cli")),
        "the import must record where the credential came from: {imported}"
    );
    let profile_id = imported
        .get("profileId")
        .and_then(Value::as_str)
        .expect("an imported profile id")
        .to_string();
    assert!(!profile_id.is_empty());

    // The credential really landed: the status RPC now reports connected, with
    // the same profile.
    let status = harness
        .rpc(76_005, "openhuman.inference_openai_oauth_status", json!({}))
        .await;
    let status = payload(&status, "openai_oauth_status after import");
    assert_eq!(
        status.get("connected"),
        Some(&json!(true)),
        "an import that does not persist is not an import: {status}"
    );
    assert_eq!(status.get("profileId"), Some(&json!(profile_id)));
    assert_eq!(status.get("authMethod"), Some(&json!("oauth")));
    assert!(
        status.get("expiresAt").is_some(),
        "the JWT `exp` must be carried onto the stored token set: {status}"
    );

    // Clean up the stored credential so no later suite in this binary inherits
    // a connected OpenAI OAuth profile.
    let _ = harness
        .rpc(
            76_006,
            "openhuman.inference_openai_oauth_disconnect",
            json!({}),
        )
        .await;

    harness.join.abort();
}
