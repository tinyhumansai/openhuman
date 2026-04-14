//! Core CLI auth flows: load config, branch `app-session` vs provider storage.

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::credentials::rpc;
use crate::openhuman::credentials::APP_SESSION_PROVIDER;

pub fn parse_field_equals_entries(entries: &[String]) -> Result<serde_json::Value, String> {
    let mut fields = serde_json::Map::new();
    for entry in entries {
        let Some((raw_key, raw_value)) = entry.split_once('=') else {
            return Err(format!(
                "invalid --field value '{entry}', expected key=value format"
            ));
        };
        let key = raw_key.trim();
        if key.is_empty() {
            return Err("invalid --field value with empty key".to_string());
        }
        fields.insert(
            key.to_string(),
            serde_json::Value::String(raw_value.to_string()),
        );
    }
    Ok(serde_json::Value::Object(fields))
}

pub async fn cli_auth_login(
    provider: String,
    token: String,
    user_id: Option<String>,
    user_json: Option<serde_json::Value>,
    fields: serde_json::Value,
    profile: Option<String>,
    set_active: bool,
) -> Result<serde_json::Value, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let provider = provider.trim().to_string();

    if provider == APP_SESSION_PROVIDER {
        rpc::store_session(&config, &token, user_id, user_json)
            .await?
            .into_cli_compatible_json()
    } else {
        let fields_opt = match &fields {
            serde_json::Value::Object(map) if map.is_empty() => None,
            _ => Some(fields),
        };
        rpc::store_provider_credentials(
            &config,
            &provider,
            profile.as_deref(),
            Some(token),
            fields_opt,
            Some(set_active),
        )
        .await?
        .into_cli_compatible_json()
    }
}

pub async fn cli_auth_logout(
    provider: String,
    profile: Option<String>,
) -> Result<serde_json::Value, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let provider = provider.trim().to_string();
    if provider == APP_SESSION_PROVIDER {
        rpc::clear_session(&config)
            .await?
            .into_cli_compatible_json()
    } else {
        rpc::remove_provider_credentials(&config, &provider, profile.as_deref())
            .await?
            .into_cli_compatible_json()
    }
}

pub async fn cli_auth_status(
    provider: String,
    _profile: Option<String>,
) -> Result<serde_json::Value, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let provider = provider.trim().to_string();
    if provider == APP_SESSION_PROVIDER {
        rpc::auth_get_state(&config)
            .await?
            .into_cli_compatible_json()
    } else {
        rpc::list_provider_credentials(&config, Some(provider))
            .await?
            .into_cli_compatible_json()
    }
}

pub async fn cli_auth_list(provider_filter: Option<String>) -> Result<serde_json::Value, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let filter = provider_filter
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    rpc::list_provider_credentials(&config, filter)
        .await?
        .into_cli_compatible_json()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::TEST_ENV_LOCK as ENV_LOCK;
    use tempfile::TempDir;

    fn set_workspace(tmp: &TempDir) {
        // SAFETY: env mutation is guarded by ENV_LOCK which every test in
        // this module acquires before touching OPENHUMAN_WORKSPACE.
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
        }
    }

    fn clear_workspace() {
        unsafe {
            std::env::remove_var("OPENHUMAN_WORKSPACE");
        }
    }

    // ── parse_field_equals_entries ──────────────────────────────────

    #[test]
    fn parse_field_equals_entries_builds_json_object_from_key_eq_value() {
        let v =
            parse_field_equals_entries(&["api_key=sk-abc".into(), "org_id=org-42".into()]).unwrap();
        assert_eq!(v["api_key"], "sk-abc");
        assert_eq!(v["org_id"], "org-42");
    }

    #[test]
    fn parse_field_equals_entries_returns_empty_object_for_empty_list() {
        let v = parse_field_equals_entries(&[]).unwrap();
        assert!(v.is_object());
        assert!(v.as_object().unwrap().is_empty());
    }

    #[test]
    fn parse_field_equals_entries_preserves_value_with_equals_signs() {
        // Only the first `=` is the separator — subsequent `=` are value chars.
        let v = parse_field_equals_entries(&["token=a=b=c".into()]).unwrap();
        assert_eq!(v["token"], "a=b=c");
    }

    #[test]
    fn parse_field_equals_entries_trims_key_whitespace() {
        let v = parse_field_equals_entries(&["  api_key  =sk".into()]).unwrap();
        assert_eq!(v["api_key"], "sk");
    }

    #[test]
    fn parse_field_equals_entries_allows_empty_value() {
        let v = parse_field_equals_entries(&["api_key=".into()]).unwrap();
        assert_eq!(v["api_key"], "");
    }

    #[test]
    fn parse_field_equals_entries_rejects_entry_without_equals() {
        let err = parse_field_equals_entries(&["noequalsign".into()]).unwrap_err();
        assert!(err.contains("key=value"));
    }

    #[test]
    fn parse_field_equals_entries_rejects_empty_key() {
        let err = parse_field_equals_entries(&["=value".into()]).unwrap_err();
        assert!(err.contains("empty key"));
        let err = parse_field_equals_entries(&["   =value".into()]).unwrap_err();
        assert!(err.contains("empty key"));
    }

    // ── cli_auth_* end-to-end ─────────────────────────────────────
    //
    // These tests exercise the branch logic inside each CLI entrypoint
    // by pointing `OPENHUMAN_WORKSPACE` at a temp dir and relying on
    // `load_config_with_timeout()` to resolve from that override.

    #[tokio::test]
    async fn cli_auth_login_provider_branch_stores_credentials() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = TempDir::new().unwrap();
        set_workspace(&tmp);
        let result = cli_auth_login(
            "openai".into(),
            "sk-test".into(),
            None,
            None,
            serde_json::Value::Object(serde_json::Map::new()),
            None,
            true,
        )
        .await;
        clear_workspace();
        let out = result.expect("login should succeed for provider branch");
        assert!(
            out.to_string().contains("openai"),
            "unexpected result: {out}"
        );
    }

    #[tokio::test]
    async fn cli_auth_login_with_non_empty_fields_passes_them_through() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = TempDir::new().unwrap();
        set_workspace(&tmp);
        let fields = serde_json::json!({ "org_id": "org-1" });
        let result =
            cli_auth_login("openai".into(), "sk".into(), None, None, fields, None, true).await;
        clear_workspace();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn cli_auth_logout_provider_branch_reports_no_op_on_empty_store() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = TempDir::new().unwrap();
        set_workspace(&tmp);
        let result = cli_auth_logout("openai".into(), None).await;
        clear_workspace();
        let out = result.expect("logout branch must resolve ok");
        // `remove_provider_credentials` returns `{removed: false}` when the
        // profile never existed; the CLI envelope nests it under `result`.
        let s = out.to_string();
        assert!(s.contains("removed"), "unexpected: {s}");
    }

    #[tokio::test]
    async fn cli_auth_status_provider_branch_lists_for_provider() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = TempDir::new().unwrap();
        set_workspace(&tmp);
        let result = cli_auth_status("openai".into(), None).await;
        clear_workspace();
        let out = result.expect("status must succeed on empty store");
        // Empty — just sanity-check shape.
        assert!(
            out.is_object() || out.is_array(),
            "unexpected status shape: {out}"
        );
    }

    #[tokio::test]
    async fn cli_auth_list_with_empty_filter_lists_all() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = TempDir::new().unwrap();
        set_workspace(&tmp);
        let out = cli_auth_list(None).await.expect("list ok");
        clear_workspace();
        // Fresh store → empty list wrapped in the usual logs envelope.
        assert!(out.is_object() || out.is_array(), "unexpected: {out}");
    }

    #[tokio::test]
    async fn cli_auth_list_rejects_whitespace_only_filter_as_no_filter() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = TempDir::new().unwrap();
        set_workspace(&tmp);
        let out = cli_auth_list(Some("   ".into())).await.expect("list ok");
        clear_workspace();
        assert!(out.is_object() || out.is_array());
    }
}
