use super::resolve_local_runtime_key;
use crate::config::Config;

#[test]
fn omlx_key_falls_back_to_local_ai_api_key() {
    let mut config = Config::default();
    config.local_ai.api_key = Some("  sk-omlx-list  ".into());
    assert_eq!(
        resolve_local_runtime_key("omlx", String::new(), &config),
        "sk-omlx-list"
    );
}

#[test]
fn looked_up_key_wins_over_local_ai() {
    let mut config = Config::default();
    config.local_ai.api_key = Some("sk-local".into());
    assert_eq!(
        resolve_local_runtime_key("omlx", "from-profiles".into(), &config),
        "from-profiles"
    );
}

#[test]
fn non_omlx_slug_does_not_fall_back() {
    let mut config = Config::default();
    config.local_ai.api_key = Some("sk-local".into());
    assert_eq!(
        resolve_local_runtime_key("ollama", String::new(), &config),
        ""
    );
}

fn workspace_with_provider(
    tmp: &tempfile::TempDir,
    slug: &str,
    endpoint: &str,
    auth_style: crate::config::schema::cloud_providers::AuthStyle,
) -> Config {
    use crate::config::schema::cloud_providers::CloudProviderCreds;
    let mut config = Config {
        config_path: tmp.path().join("config.toml"),
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        ..Config::default()
    };
    config.secrets.encrypt = false;
    config.cloud_providers.push(CloudProviderCreds {
        id: format!("p_{slug}"),
        slug: slug.to_string(),
        label: slug.to_string(),
        endpoint: endpoint.to_string(),
        auth_style,
        default_model: None,
    });
    config
}

/// Sentry TAURI-RUST-114C/114D: the app stores Claude Code as a cloud provider
/// with the cosmetic endpoint `cli://claude-code`. Probing `{endpoint}/models`
/// failed in reqwest's builder ("builder error for url (cli://claude-code/models)")
/// and reached Sentry for every picker open. Claude Code runs as a local CLI
/// and has no `/models` listing, so the catalog is empty — not an error.
#[tokio::test]
async fn claude_code_cli_provider_lists_no_models_without_a_request() {
    use crate::config::schema::cloud_providers::AuthStyle;
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = workspace_with_provider(&tmp, "claude-code", "cli://claude-code", AuthStyle::None);

    let outcome = super::list_configured_models_from_config("claude-code", &config)
        .await
        .expect("claude-code must list an empty catalog, not fail");
    assert_eq!(outcome.value["models"], serde_json::json!([]));
}

/// A `cli://` placeholder endpoint cannot host a `/models` listing;
/// it degrades to an empty catalog instead of a reqwest builder error.
#[tokio::test]
async fn non_http_endpoint_lists_no_models_without_a_request() {
    use crate::config::schema::cloud_providers::AuthStyle;
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = workspace_with_provider(&tmp, "custom-cli", "cli://custom", AuthStyle::None);

    let outcome = super::list_configured_models_from_config("custom-cli", &config)
        .await
        .expect("non-http endpoint must list an empty catalog, not fail");
    assert_eq!(outcome.value["models"], serde_json::json!([]));
}

/// Only the `cli://` placeholder is exempt: any other non-http scheme is a
/// misconfigured provider and must surface as an error, not an empty catalog.
#[tokio::test]
async fn unsupported_scheme_endpoint_still_surfaces_an_error() {
    use crate::config::schema::cloud_providers::AuthStyle;
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = workspace_with_provider(
        &tmp,
        "custom-ftp",
        "ftp://provider.example",
        AuthStyle::None,
    );

    let result = super::list_configured_models_from_config("custom-ftp", &config).await;
    assert!(result.is_err(), "ftp:// must not read as an empty catalog");
}

/// With no catalog provider registered, the managed provider (by id, and by an
/// `OpenhumanJwt` entry) lists an empty catalog rather than failing.
#[tokio::test]
async fn unregistered_managed_catalog_lists_no_models() {
    use crate::config::schema::cloud_providers::AuthStyle;
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = workspace_with_provider(
        &tmp,
        "custom-managed",
        "https://managed.example/v1",
        AuthStyle::OpenhumanJwt,
    );
    for provider in ["openhuman", "custom-managed"] {
        let outcome = super::list_configured_models_from_config(provider, &config)
            .await
            .expect("an unregistered managed catalog is empty, not an error");
        assert_eq!(outcome.value["models"], serde_json::json!([]), "{provider}");
    }
}
