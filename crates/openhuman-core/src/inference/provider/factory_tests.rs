use super::*;
use crate::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};
use crate::config::Config;
use crate::credentials::AuthService;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn with_library_test_context<T>(f: impl FnOnce() -> T) -> T {
    let context = crate::core::runtime::CoreContext::for_test_with_host_kind(
        crate::core::types::HostKind::Library,
        crate::core::runtime::DomainSet::full(),
        None,
    );
    crate::core::runtime::CoreContext::sync_scope(context, f)
}

async fn with_library_test_context_async<F: std::future::Future>(
    f: impl FnOnce() -> F,
) -> F::Output {
    let context = crate::core::runtime::CoreContext::for_test_with_host_kind(
        crate::core::types::HostKind::Library,
        crate::core::runtime::DomainSet::full(),
        None,
    );
    crate::core::runtime::CoreContext::scope(context, f()).await
}

struct ManagedRouteFixtureModel(tinyinference_llm::model::ModelProfile);

#[async_trait::async_trait]
impl ChatModel<()> for ManagedRouteFixtureModel {
    fn profile(&self) -> Option<&tinyinference_llm::model::ModelProfile> {
        Some(&self.0)
    }

    async fn invoke(
        &self,
        _state: &(),
        _request: tinyinference_llm::model::ModelRequest,
    ) -> tinyinference_llm::Result<tinyinference_llm::model::ModelResponse> {
        Ok(tinyinference_llm::model::ModelResponse::assistant(
            "fixture",
        ))
    }
}

fn install_test_managed_provider() -> Arc<std::sync::Mutex<Vec<bool>>> {
    use crate::config::schema::cloud_providers::AuthStyle;
    use std::sync::Mutex;
    use tinyinference_llm::providers::overrides::{ChatModelEntry, ProviderModelMetadata};

    let native_modes = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&native_modes);
    crate::inference::registry::register_chat_model(
        crate::inference::provider::factory::PROVIDER_OPENHUMAN,
        ChatModelEntry {
            build: Arc::new(move |request| {
                seen.lock().unwrap().push(request.native_tool_calling);
                let mut profile = tinyinference_llm::model::ModelProfile::default();
                profile.provider = Some("managed".to_string());
                profile.tool_calling = request.native_tool_calling;
                profile.parallel_tool_calls = request.native_tool_calling;
                profile.streaming_tool_chunks = request.native_tool_calling;
                Ok((
                    Arc::new(ManagedRouteFixtureModel(profile)),
                    request
                        .model_override
                        .or(request.default_model)
                        .unwrap_or("fixture/managed-default")
                        .to_string(),
                ))
            }),
            probe: None,
            models: Some(ProviderModelMetadata {
                default_model: Arc::new(|| Some("fixture/managed-default".into())),
                role_for_model: Arc::new(|model| {
                    (model == "hint:reasoning").then(|| "reasoning".into())
                }),
                is_alias: Arc::new(|model| model == "hint:reasoning"),
                is_raw_model: Arc::new(|model| !model.starts_with("hint:")),
                supports_vision: Arc::new(|_| false),
                resolve_model: Arc::new(|_, config, model| match model {
                    Some("hint:reasoning") => Some("fixture/managed-default".into()),
                    Some("reasoning-v1") => Some("fixture/managed-default".into()),
                    Some(model) if !model.starts_with("hint:") => Some(model.into()),
                    Some(_) => None,
                    None => config
                        .config
                        .default_model
                        .clone()
                        .or_else(|| Some("fixture/managed-default".into())),
                }),
                context_window: Arc::new(|_| None),
                default_pricing: Arc::new(|| None),
                without_reasoning: Arc::new(|request| request),
            }),
        },
    );
    crate::config::schema::cloud_providers::register_hosted_cloud_provider_policy(
        Arc::new(|slug| slug == "fixture-managed"),
        Arc::new(|style| style == AuthStyle::OpenhumanJwt),
    );
    native_modes
}

fn fixture_managed_entry(id: &str) -> CloudProviderCreds {
    CloudProviderCreds {
        id: id.to_string(),
        slug: "fixture-managed".to_string(),
        label: "Managed fixture".to_string(),
        endpoint: "https://fixture.managed.test/v1".to_string(),
        auth_style: AuthStyle::OpenhumanJwt,
        default_model: Some("fixture/managed-default".into()),
    }
}

use crate::inference::provider::factory::access_gates::host_requires_session;

#[tokio::test]
async fn inspection_codex_oauth_metadata_marks_output_budget_unsupported() {
    use crate::inference::provider::factory::InspectionOutputBudgetCapability;
    use tinyinference_llm::providers::openai::codex::OpenAiCodexRouting;

    let codex = OpenAiCodexRouting {
        endpoint: "https://chatgpt.com/backend-api/codex".into(),
        using_oauth: true,
        account_id: Some("acct-test".into()),
    };
    assert_eq!(
        super::cloud_slug::inspection_output_budget_capability(&codex),
        InspectionOutputBudgetCapability::Unsupported
    );
    assert_eq!(
        super::cloud_slug::inspection_output_budget_capability(&OpenAiCodexRouting::standard(
            "https://api.openai.com/v1"
        )),
        InspectionOutputBudgetCapability::Bounded
    );
}

async fn create_test_chat_model_from_string(
    role: &str,
    provider: &str,
    config: &Config,
) -> anyhow::Result<(Arc<dyn ChatModel<()>>, String)> {
    with_library_test_context_async(|| async {
        create_chat_model_from_string_with_model_id(role, provider, config, 0.7).await
    })
    .await
}

async fn create_test_local_chat_model_from_string(
    provider: &str,
    config: &Config,
) -> anyhow::Result<(Arc<dyn ChatModel<()>>, String)> {
    create_local_chat_model_from_string(provider, config)
}

async fn create_test_omlx_model(
    model: &str,
    _temperature: Option<f64>,
    config: &Config,
) -> anyhow::Result<(Arc<dyn ChatModel<()>>, String)> {
    create_local_chat_model_from_string(&format!("omlx:{model}"), config)
}

fn config_with_providers(providers: Vec<CloudProviderCreds>) -> Config {
    let mut c = Config::default();
    c.cloud_providers = providers;
    c
}

fn config_with_providers_in_tempdir(tmp: &TempDir, providers: Vec<CloudProviderCreds>) -> Config {
    let mut c = config_with_providers(providers);
    c.workspace_dir = tmp.path().join("workspace");
    c.config_path = tmp.path().join("config.toml");
    c
}

#[tokio::test]
async fn inspection_requires_credentials_for_explicit_authenticated_cloud_routes() {
    with_library_test_context_async(|| async {
        let tmp = TempDir::new().expect("temporary provider workspace");
        let mut provider = openai_entry("p_missing_key", "factory-missing-key-test");
        let mut config = config_with_providers_in_tempdir(&tmp, vec![provider.clone()]);
        config.chat_provider = Some("factory-missing-key-test:gpt-4o".to_string());

        let inspection = super::inspection::create_chat_model_with_model_id_in_for_inspection(
            &crate::core::runtime::DispatchCtx::ambient(),
            "chat",
            &config,
            0.0,
        )
        .await;
        assert!(
            inspection.is_err(),
            "inspection must reject explicit Bearer routes without credentials"
        );

        let runtime = super::chat_model::create_chat_model_with_model_id_in(
            &crate::core::runtime::DispatchCtx::ambient(),
            "chat",
            &config,
            0.0,
        )
        .await;
        assert!(
            runtime.is_ok(),
            "runtime construction preserves explicit routes before a key is saved: {:?}",
            runtime.err()
        );

        provider.auth_style = AuthStyle::Anthropic;
        config.cloud_providers = vec![provider.clone()];
        assert!(
            super::inspection::create_chat_model_with_model_id_in_for_inspection(
                &crate::core::runtime::DispatchCtx::ambient(),
                "chat",
                &config,
                0.0,
            )
            .await
            .is_err(),
            "inspection must reject explicit Anthropic routes without credentials"
        );

        provider.auth_style = AuthStyle::None;
        config.cloud_providers = vec![provider];
        assert!(
            super::inspection::create_chat_model_with_model_id_in_for_inspection(
                &crate::core::runtime::DispatchCtx::ambient(),
                "chat",
                &config,
                0.0,
            )
            .await
            .is_ok(),
            "unauthenticated proxy routes do not require a stored credential"
        );
    })
    .await;
}

fn oh_entry(id: &str) -> CloudProviderCreds {
    CloudProviderCreds {
        id: id.to_string(),
        slug: "openhuman".to_string(),
        label: "OpenHuman".to_string(),
        endpoint: "https://api.openhuman.ai/v1".to_string(),
        auth_style: AuthStyle::OpenhumanJwt,
        ..Default::default()
    }
}

fn openai_entry(id: &str, slug: &str) -> CloudProviderCreds {
    CloudProviderCreds {
        id: id.to_string(),
        slug: slug.to_string(),
        label: "OpenAI".to_string(),
        endpoint: "https://api.openai.com/v1".to_string(),
        auth_style: AuthStyle::Bearer,
        default_model: Some("gpt-4o".to_string()),
    }
}

fn anthropic_entry(id: &str, slug: &str) -> CloudProviderCreds {
    CloudProviderCreds {
        id: id.to_string(),
        slug: slug.to_string(),
        label: "Anthropic".to_string(),
        endpoint: "https://api.anthropic.com/v1".to_string(),
        auth_style: AuthStyle::Anthropic,
        default_model: Some("claude-sonnet-4-6".to_string()),
    }
}

fn nvidia_nim_entry(id: &str, default_model: Option<&str>) -> CloudProviderCreds {
    CloudProviderCreds {
        id: id.to_string(),
        slug: "nvidia-nim".to_string(),
        label: "NVIDIA NIM".to_string(),
        endpoint: "https://integrate.api.nvidia.com/v1".to_string(),
        auth_style: AuthStyle::Bearer,
        default_model: default_model.map(ToString::to_string),
    }
}

// ── config.api_key fallback scoping (PR #2724) ───────────────────────────

/// Build a tempdir-backed Config with a global `config.api_key`, a custom
/// `inference_url`, and two cloud providers: one whose endpoint matches the
/// inference_url (the legacy direct-inference slug) and one that does not.
///
/// The tempdir workspace has no stored auth-profiles, so `lookup_key_for_slug`
/// exhausts the standard auth path and reaches the `config.api_key` fallback.
fn config_for_api_key_fallback(tmp: &TempDir) -> Config {
    let mut custom = openai_entry("p_custom", "custom");
    custom.endpoint = "https://inference.example.com/v1".to_string();
    let config = config_with_providers_in_tempdir(
        tmp,
        vec![custom, anthropic_entry("p_anthropic", "anthropic")],
    );
    let mut config = config;
    config.api_key = Some("global-key".to_string());
    config.inference_url = Some("https://inference.example.com/v1".to_string());
    config
}

// ── #3767: managed-credits gate bypass (gate-only, per-tier) ───────────────
//
// Routing is NOT changed by this fix — selecting a BYO provider already routes
// inference correctly. The gate is evaluated PER TIER so the UI checks whichever
// tier the user actually selected: the chat header's "Quick" mode runs on the
// `chat` tier and "Reasoning" mode on the `reasoning` tier. `role_bypasses_
// managed_credits(role)` is true when that role runs on the user's own funding
// (a BYO cloud key, a local runtime, or claude-code) with usable credentials.
// Tiers that stay managed and run anyway surface the per-call 402 error.

/// Store a usable provider key under the new-style `provider:<slug>` profile so
/// `lookup_key_for_slug` resolves it.
async fn store_byo_key(config: &Config, slug: &str, token: &str) {
    let auth = AuthService::from_config(config);
    auth.store_provider_token(
        &format!("provider:{slug}"),
        "default",
        token,
        Default::default(),
        true,
    )
    .await
    .expect("store provider token");
}

#[tokio::test(flavor = "multi_thread")]
async fn inspection_construction_does_not_refresh_unselected_expired_codex_oauth_profile() {
    use crate::config::test_env::EnvVarGuard;
    use crate::credentials::openai_oauth::{OPENAI_OAUTH_PROFILE_NAME, OPENAI_PROVIDER_KEY};
    use chrono::{Duration, Utc};

    let _inference_guard = crate::inference::inference_test_guard_async().await;
    let tmp = tempfile::tempdir().expect("temporary credentials fixture");
    let refresh_server = MockServer::start().await;
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().join("workspace");
    config.action_dir = tmp.path().join("action");
    config.secrets.encrypt = false;

    let _env = EnvVarGuard::locked_async()
        .await
        .with("HOME", tmp.path())
        .with("OPENHUMAN_WORKSPACE", &config.workspace_dir)
        .with("OPENHUMAN_KEYRING_BACKEND", "file")
        .with(
            "OPENAI_CODEX_OAUTH_TOKEN_URL",
            format!("{}/token", refresh_server.uri()),
        );
    let expires_at = (Utc::now() - Duration::minutes(5)).to_rfc3339();
    let created_at = (Utc::now() - Duration::hours(1)).to_rfc3339();
    let updated_at = created_at.clone();
    let profiles_path = tmp.path().join("auth-profiles.json");
    std::fs::write(
        &profiles_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": 1,
            "updated_at": created_at.clone(),
            "active_profiles": {
                (OPENAI_PROVIDER_KEY): format!("{OPENAI_PROVIDER_KEY}:default")
            },
            "profiles": {
                (format!("{OPENAI_PROVIDER_KEY}:default")): {
                    "provider": OPENAI_PROVIDER_KEY,
                    "profile_name": "default",
                    "kind": "token",
                    "token": "dummy-selected-api-key",
                    "created_at": created_at.clone(),
                    "updated_at": updated_at.clone(),
                    "metadata": {}
                },
                (format!("{OPENAI_PROVIDER_KEY}:{OPENAI_OAUTH_PROFILE_NAME}")): {
                    "provider": OPENAI_PROVIDER_KEY,
                    "profile_name": OPENAI_OAUTH_PROFILE_NAME,
                    "kind": "oauth",
                    "access_token": "expired-oauth-access-token",
                    "refresh_token": "dummy-refresh-token",
                    "expires_at": expires_at,
                    "token_type": "Bearer",
                    "created_at": created_at.clone(),
                    "updated_at": updated_at.clone(),
                    "metadata": {}
                }
            }
        }))
        .expect("serialize raw auth-profile schema"),
    )
    .expect("write raw profile fixture without mutable auth APIs");

    let selected = AuthService::get_profile_read_only(&config, OPENAI_PROVIDER_KEY, None)
        .await
        .expect("selected profile is readable without mutable auth")
        .expect("selected API-key profile exists");
    assert_eq!(selected.profile_name, "default");
    assert_eq!(selected.token.as_deref(), Some("dummy-selected-api-key"));
    let oauth = AuthService::get_profile_read_only(
        &config,
        OPENAI_PROVIDER_KEY,
        Some(OPENAI_OAUTH_PROFILE_NAME),
    )
    .await
    .expect("named OAuth profile is readable without refreshing")
    .expect("expired OAuth profile exists");
    assert!(oauth
        .token_set
        .as_ref()
        .and_then(|token_set| token_set.expires_at)
        .is_some_and(|expires_at| expires_at < Utc::now()));
    let selected_codex_credential =
        super::credentials::lookup_key_for_slug_read_only("openai", &config)
            .await
            .expect("read selected OpenAI API-key credential");
    assert_eq!(selected_codex_credential.key, "dummy-selected-api-key");
    assert!(!selected_codex_credential.is_oauth);
    assert_eq!(selected_codex_credential.account_id, None);

    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "refreshed-oauth-token-must-not-be-written",
            "refresh_token": "rotated-dummy-refresh-token",
            "expires_in": 3600,
        })))
        .mount(&refresh_server)
        .await;
    let snapshot =
        std::fs::read(&profiles_path).expect("snapshot raw credential file before inspection");

    let mut provider = openai_entry("p_openai_inspection", "openai");
    provider.endpoint = "https://api.openai.example.test/v1".into();
    config.cloud_providers = vec![provider];
    config.chat_provider = Some("openai:gpt-4o".into());
    let inspected = super::inspection::create_chat_model_with_model_id_in_for_inspection(
        &crate::core::runtime::DispatchCtx::ambient(),
        "chat",
        &config,
        0.0,
    )
    .await;
    assert!(
        inspected.is_ok(),
        "the selected API-key profile is sufficient for model construction"
    );

    assert!(
        refresh_server
            .received_requests()
            .await
            .expect("refresh mock records requests")
            .is_empty(),
        "inspection construction must not refresh the unrelated expired OAuth profile"
    );
    assert_eq!(
        std::fs::read(tmp.path().join("auth-profiles.json"))
            .expect("read credential store after inspection"),
        snapshot,
        "inspection must leave all credential bytes unchanged"
    );
    let selected_after = AuthService::get_profile_read_only(&config, OPENAI_PROVIDER_KEY, None)
        .await
        .expect("selected profile remains readable")
        .expect("selected profile remains present");
    assert_eq!(selected_after.profile_name, "default");
    assert_eq!(
        selected_after.token.as_deref(),
        Some("dummy-selected-api-key")
    );
    let oauth_after = AuthService::get_profile_read_only(
        &config,
        OPENAI_PROVIDER_KEY,
        Some(OPENAI_OAUTH_PROFILE_NAME),
    )
    .await
    .expect("OAuth snapshot remains readable")
    .expect("OAuth snapshot remains present");
    assert_eq!(
        oauth_after
            .token_set
            .as_ref()
            .map(|token_set| token_set.access_token.as_str()),
        Some("expired-oauth-access-token"),
        "inspection must not replace an expired token with a refreshed one"
    );
    assert!(oauth_after
        .token_set
        .as_ref()
        .and_then(|token_set| token_set.expires_at)
        .is_some_and(|expires_at| expires_at < Utc::now()));
}

#[tokio::test(flavor = "multi_thread")]
async fn inspection_codex_metadata_matches_the_selected_oauth_profile() {
    use crate::config::test_env::EnvVarGuard;
    use chrono::{Duration, Utc};

    let _inference_guard = crate::inference::inference_test_guard_async().await;
    let temp = tempfile::tempdir().expect("temporary credentials fixture");
    let mut config = Config::default();
    config.config_path = temp.path().join("config.toml");
    config.workspace_dir = temp.path().join("workspace");
    config.action_dir = temp.path().join("action");
    config.secrets.encrypt = false;
    let _env = EnvVarGuard::locked_async()
        .await
        .with("HOME", temp.path())
        .with("OPENHUMAN_WORKSPACE", &config.workspace_dir)
        .with("OPENHUMAN_KEYRING_BACKEND", "file");

    let now = Utc::now();
    let created = (now - Duration::hours(1)).to_rfc3339();
    let expires = (now + Duration::hours(1)).to_rfc3339();
    let raw_profiles = serde_json::json!({
        "schema_version": 1,
        "updated_at": created,
        "active_profiles": {
            "provider:openai": "provider:openai:active-oauth"
        },
        "profiles": {
            "provider:openai:active-oauth": {
                "provider": "provider:openai",
                "profile_name": "active-oauth",
                "kind": "oauth",
                "access_token": "active-oauth-bearer",
                "refresh_token": "active-refresh-token",
                "expires_at": expires,
                "token_type": "Bearer",
                "metadata": {"account_id": "acct-active"},
                "created_at": created,
                "updated_at": created
            },
            "provider:openai:oauth": {
                "provider": "provider:openai",
                "profile_name": "oauth",
                "kind": "oauth",
                "access_token": "other-oauth-bearer",
                "refresh_token": "other-refresh-token",
                "expires_at": expires,
                "token_type": "Bearer",
                "metadata": {"account_id": "acct-unselected"},
                "created_at": created,
                "updated_at": created
            }
        }
    });
    let profiles_path = temp.path().join("auth-profiles.json");
    std::fs::write(
        &profiles_path,
        serde_json::to_vec_pretty(&raw_profiles).expect("serialize profile fixture"),
    )
    .expect("write read-only profile fixture");
    let before = std::fs::read(&profiles_path).expect("snapshot profile bytes");

    let selected = super::credentials::lookup_key_for_slug_read_only("openai", &config)
        .await
        .expect("read selected OAuth profile");
    assert_eq!(selected.key, "active-oauth-bearer");
    assert!(selected.is_oauth);
    assert_eq!(selected.account_id.as_deref(), Some("acct-active"));

    let routing =
        crate::inference::provider::openai_codex::resolve_openai_codex_routing_for_inspection(
            "openai",
            "https://api.openai.example.test/v1",
            selected.is_oauth,
            selected.account_id,
        );
    assert!(routing.using_oauth);
    assert_eq!(routing.account_id.as_deref(), Some("acct-active"));
    assert_eq!(
        std::fs::read(profiles_path).expect("profile bytes remain readable"),
        before,
        "inspection routing must not refresh or migrate profile data"
    );
}

// ── Motion B (#4727 Phase 3): wire-equivalent BYOK cloud-slug cutover ────────

fn deepseek_entry(id: &str) -> CloudProviderCreds {
    CloudProviderCreds {
        id: id.to_string(),
        slug: "deepseek".to_string(),
        label: "DeepSeek".to_string(),
        endpoint: "https://api.deepseek.com/v1".to_string(),
        auth_style: AuthStyle::Bearer,
        default_model: Some("deepseek-chat".to_string()),
    }
}

// ── Per-call inference route ────────────────────────────────────────────────
//
// These close the loop between `ephemeral_route::apply` and the two resolution
// steps that have to honour it for a routed turn to reach the caller's
// endpoint. Both are read from `Config`, so they are exercised without a
// network or a booted core.

/// A config carrying a resolved model, as `agent_chat` leaves it after applying
/// `model_override` and before building the agent.
fn routed_config(endpoint: &str, api_key: &str, model: &str) -> Config {
    use crate::config::schema::ephemeral_route::{apply, EphemeralRoute};
    let mut config = Config {
        default_model: Some(model.to_string()),
        ..Config::default()
    };
    apply(
        &mut config,
        EphemeralRoute {
            endpoint: endpoint.to_string(),
            api_key: api_key.to_string(),
            headers: Vec::new(),
            ..EphemeralRoute::default()
        },
    );
    config
}

#[tokio::test]
async fn only_library_hosts_are_exempt_from_app_login() {
    use crate::core::types::HostKind;

    assert!(!host_requires_session(HostKind::Library));
    for host in [HostKind::TauriShell, HostKind::Cli, HostKind::Docker] {
        assert!(host_requires_session(host));
    }
}

#[path = "factory_crate_native_diagnostics_tests.rs"]
mod crate_native_diagnostics_tests;
#[path = "factory_crate_native_jwt_tests.rs"]
mod crate_native_jwt_tests;
#[path = "factory_crate_native_tests.rs"]
mod crate_native_tests;
#[path = "factory_egress_fallback_tests.rs"]
mod egress_fallback_tests;
#[path = "factory_explicit_ctx_tests.rs"]
mod explicit_ctx_tests;
#[path = "factory_local_only_endpoint_tests.rs"]
mod local_only_endpoint_tests;
#[path = "factory_managed_ledger_tests.rs"]
mod managed_ledger_tests;
#[path = "factory_model_gate_tests.rs"]
mod model_gate_tests;
#[path = "factory_route_resolution_tests.rs"]
mod route_resolution_tests;
