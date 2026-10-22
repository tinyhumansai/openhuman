//! Tests for the OCI Generative AI cloud slug.
//!
//! No key material is committed: the OCI config's `key_file` holds a
//! placeholder. No test here touches the network: models are only
//! constructed.

use super::*;
use crate::config::schema::cloud_providers::OciAuthMode;

/// Contents of the profiles' `key_file`. Loading a profile only reads the
/// file; a key is parsed when a signed model is built, which these tests do
/// only through the session-token profile (its key is read per request).
const KEY_FILE_CONTENTS: &str = "placeholder key file, never parsed by these tests";

fn oci_entry(settings: OciProviderSettings) -> CloudProviderCreds {
    CloudProviderCreds {
        id: "p_oci".into(),
        slug: "oci".into(),
        label: "OCI Generative AI".into(),
        endpoint: "https://inference.generativeai.us-chicago-1.oci.oraclecloud.com".into(),
        auth_style: AuthStyle::Oci,
        oci: Some(settings),
        ..Default::default()
    }
}

fn api_key_settings() -> OciProviderSettings {
    OciProviderSettings {
        auth_mode: OciAuthMode::ApiKey,
        region: Some("us-chicago-1".into()),
        project_id: Some("ocid1.generativeaiproject.test".into()),
        ..Default::default()
    }
}

/// A temp `~/.oci`-shaped directory: a DEFAULT API-key profile, and a SESSION
/// profile that points at (not-yet-existing) token files.
fn oci_dir() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let key = dir.path().join("key.pem");
    std::fs::write(&key, KEY_FILE_CONTENTS).expect("write key");
    let config = dir.path().join("config");
    std::fs::write(
        &config,
        format!(
            "[DEFAULT]\nuser=ocid1.user.test\nfingerprint=aa:bb:cc\ntenancy=ocid1.tenancy.test\n\
             region=eu-frankfurt-1\nkey_file={key}\n\n\
             [SESSION]\nsecurity_token_file={token}\nkey_file={key}\nregion=us-ashburn-1\n",
            key = key.display(),
            token = dir.path().join("token").display(),
        ),
    )
    .expect("write config");
    (dir, config)
}

fn profile_settings(
    mode: OciAuthMode,
    config: &std::path::Path,
    profile: &str,
) -> OciProviderSettings {
    OciProviderSettings {
        auth_mode: mode,
        config_file: Some(config.display().to_string()),
        profile: Some(profile.into()),
        compartment_id: Some("ocid1.compartment.test".into()),
        project_id: Some("ocid1.generativeaiproject.test".into()),
        ..Default::default()
    }
}

#[test]
fn api_key_mode_builds_over_each_transport() {
    for (transport, needs_compartment) in [
        (OciTransportKind::Openai, false),
        (OciTransportKind::Actions, true),
        (OciTransportKind::NativeGeneric, true),
        (OciTransportKind::NativeCohere, true),
    ] {
        let mut settings = api_key_settings();
        settings.transport = transport;
        settings.compartment_id = needs_compartment.then(|| "ocid1.compartment.test".to_string());
        let entry = oci_entry(settings);
        build_oci_chat_model("oci", &entry, "genai-key-not-real", "openai.gpt-5.5", None)
            .unwrap_or_else(|e| panic!("{} must build: {e}", transport.as_str()));
    }
}

#[test]
fn api_key_mode_without_a_key_names_the_problem() {
    let entry = oci_entry(api_key_settings());
    let error = build_oci_chat_model("oci", &entry, "  ", "openai.gpt-5.5", None)
        .err()
        .expect("missing key must fail")
        .to_string();
    assert!(error.contains("no API key stored"), "{error}");
    assert!(error.contains("'oci'"), "{error}");
}

#[test]
fn api_key_mode_requires_a_region() {
    let mut settings = api_key_settings();
    settings.region = None;
    let error = oci_config_from_entry("oci", &settings, "k", "m").unwrap_err();
    assert!(error.contains("oci.region"), "{error}");
}

#[test]
fn native_and_actions_transports_require_a_compartment() {
    for transport in [
        OciTransportKind::Actions,
        OciTransportKind::NativeGeneric,
        OciTransportKind::NativeCohere,
    ] {
        let mut settings = api_key_settings();
        settings.transport = transport;
        let error = oci_config_from_entry("oci", &settings, "k", "m").unwrap_err();
        assert!(error.contains("oci.compartment_id"), "{error}");
        assert!(error.contains(transport.as_str()), "{error}");
    }
}

#[test]
fn signed_auth_on_the_openai_transport_requires_a_compartment() {
    let (_dir, config_file) = oci_dir();
    let mut settings = profile_settings(OciAuthMode::ConfigProfile, &config_file, "DEFAULT");
    settings.transport = OciTransportKind::Openai;
    settings.compartment_id = None;
    let error = oci_config_from_entry("oci", &settings, "", "m").unwrap_err();
    assert!(error.contains("oci.compartment_id"), "{error}");
    assert!(error.contains("auth_mode=config_profile"), "{error}");

    settings.compartment_id = Some("ocid1.compartment.test".into());
    let config = oci_config_from_entry("oci", &settings, "", "m").unwrap();
    assert_eq!(
        config.compartment_id.as_deref(),
        Some("ocid1.compartment.test")
    );
}

#[test]
fn project_and_dedicated_endpoint_are_carried_into_the_config() {
    let mut settings = api_key_settings();
    settings.project_id = Some(" ocid1.project.test ".into());
    settings.dedicated_endpoint_id = Some("ocid1.endpoint.test".into());
    let config = oci_config_from_entry("oci", &settings, "k", "cohere.command-a-03-2025").unwrap();
    assert_eq!(config.project_id.as_deref(), Some("ocid1.project.test"));
    assert_eq!(
        config.serving,
        OciServing::Dedicated {
            endpoint_id: "ocid1.endpoint.test".into()
        }
    );
    assert_eq!(config.model, "cohere.command-a-03-2025");
}

#[test]
fn entry_endpoint_overrides_the_region_host() {
    let mut entry = oci_entry(api_key_settings());
    entry.endpoint = "  https://genai.private.example.oraclecloud.com  ".into();
    let config = oci_config_from_entry("oci", &api_key_settings(), "k", "m").unwrap();
    let config = with_entry_endpoint("oci", config, &entry);
    assert_eq!(
        config.endpoint.as_deref(),
        Some("https://genai.private.example.oraclecloud.com")
    );
}

#[test]
fn the_openai_transport_requires_a_project() {
    for project in [None, Some("  ".to_string())] {
        let mut settings = api_key_settings();
        settings.project_id = project;
        let error = oci_config_from_entry("oci", &settings, "k", "m").unwrap_err();
        assert!(error.contains("oci.project_id"), "{error}");
    }
    let mut settings = api_key_settings();
    settings.project_id = None;
    settings.transport = OciTransportKind::NativeGeneric;
    settings.compartment_id = Some("ocid1.compartment.test".into());
    oci_config_from_entry("oci", &settings, "k", "m").expect("only /openai/v1 needs a project");
}

#[test]
fn the_builtin_preset_endpoint_keeps_the_region_host() {
    let mut settings = api_key_settings();
    settings.region = Some("eu-frankfurt-1".into());
    for endpoint in [
        "https://inference.generativeai.us-chicago-1.oci.oraclecloud.com",
        "https://inference.generativeai.us-chicago-1.oci.oraclecloud.com/",
    ] {
        let mut entry = oci_entry(settings.clone());
        entry.endpoint = endpoint.into();
        let config = oci_config_from_entry("oci", &settings, "k", "m").unwrap();
        let config = with_entry_endpoint("oci", config, &entry);
        assert_eq!(config.endpoint, None, "{endpoint}");
        assert_eq!(config.region.as_deref(), Some("eu-frankfurt-1"));
    }
}

#[test]
fn the_route_temperature_override_reaches_the_config() {
    let settings = api_key_settings();
    let entry = oci_entry(settings.clone());
    let config = entry_oci_config("oci", &entry, &settings, "k", "m", Some(0.2)).unwrap();
    assert_eq!(config.temperature_override, Some(0.2));
    let config = entry_oci_config("oci", &entry, &settings, "k", "m", None).unwrap();
    assert_eq!(config.temperature_override, None);
}

#[test]
fn blank_entry_endpoint_keeps_the_region_host() {
    let mut entry = oci_entry(api_key_settings());
    entry.endpoint = "   ".into();
    let config = oci_config_from_entry("oci", &api_key_settings(), "k", "m").unwrap();
    assert_eq!(with_entry_endpoint("oci", config, &entry).endpoint, None);
}

#[test]
fn entry_endpoint_override_must_be_https_or_loopback() {
    let mut entry = oci_entry(api_key_settings());
    entry.endpoint = "http://127.0.0.1:18766".into();
    build_oci_chat_model("oci", &entry, "genai-key-not-real", "openai.gpt-5.5", None)
        .unwrap_or_else(|e| panic!("a loopback capture proxy must build: {e}"));

    entry.endpoint = "http://genai.example.com".into();
    let error = build_oci_chat_model("oci", &entry, "genai-key-not-real", "openai.gpt-5.5", None)
        .err()
        .expect("plain http to a remote host must fail")
        .to_string();
    assert!(error.contains("https://"), "{error}");
    assert!(error.contains("'oci'"), "{error}");
}

#[test]
fn config_profile_reads_the_profile_and_inherits_its_region() {
    let (_dir, config) = oci_dir();
    let settings = profile_settings(OciAuthMode::ConfigProfile, &config, "DEFAULT");
    let built = oci_config_from_entry("oci", &settings, "", "openai.gpt-5.5").unwrap();
    assert_eq!(built.region.as_deref(), Some("eu-frankfurt-1"));
    assert!(matches!(built.auth, OciAuth::UserPrincipal { .. }));
}

#[test]
fn explicit_region_overrides_the_profile_region() {
    let (_dir, config) = oci_dir();
    let mut settings = profile_settings(OciAuthMode::ConfigProfile, &config, "DEFAULT");
    settings.region = Some("us-chicago-1".into());
    let built = oci_config_from_entry("oci", &settings, "", "m").unwrap();
    assert_eq!(built.region.as_deref(), Some("us-chicago-1"));
}

#[test]
fn session_token_mode_uses_the_token_file_profile() {
    let (_dir, config) = oci_dir();
    let settings = profile_settings(OciAuthMode::SessionToken, &config, "SESSION");
    let built = oci_config_from_entry("oci", &settings, "", "m").unwrap();
    assert!(matches!(built.auth, OciAuth::SessionTokenFile { .. }));
    assert_eq!(built.region.as_deref(), Some("us-ashburn-1"));
    build_oci_chat_model("oci", &oci_entry(settings), "", "openai.gpt-5.5", None).expect("builds");
}

#[test]
fn profile_kind_must_match_the_auth_mode() {
    let (_dir, config) = oci_dir();
    let error = oci_config_from_entry(
        "oci",
        &profile_settings(OciAuthMode::SessionToken, &config, "DEFAULT"),
        "",
        "m",
    )
    .unwrap_err();
    assert!(error.contains("not a session-token profile"), "{error}");
    let error = oci_config_from_entry(
        "oci",
        &profile_settings(OciAuthMode::ConfigProfile, &config, "SESSION"),
        "",
        "m",
    )
    .unwrap_err();
    assert!(
        error.contains("set oci.auth_mode to session_token"),
        "{error}"
    );
}

#[test]
fn missing_profile_and_missing_file_are_reported_with_the_provider_slug() {
    let (dir, config) = oci_dir();
    let error = oci_config_from_entry(
        "oci",
        &profile_settings(OciAuthMode::ConfigProfile, &config, "NOPE"),
        "",
        "m",
    )
    .unwrap_err();
    assert!(error.contains("'oci'") && error.contains("NOPE"), "{error}");
    let error = oci_config_from_entry(
        "oci",
        &profile_settings(
            OciAuthMode::ConfigProfile,
            &dir.path().join("absent"),
            "DEFAULT",
        ),
        "",
        "m",
    )
    .unwrap_err();
    assert!(error.contains("could not read OCI config"), "{error}");
}

#[test]
fn errors_never_contain_key_material() {
    let (_dir, config) = oci_dir();
    let error = oci_config_from_entry(
        "oci",
        &profile_settings(OciAuthMode::SessionToken, &config, "DEFAULT"),
        "",
        "m",
    )
    .unwrap_err();
    assert!(!error.contains(KEY_FILE_CONTENTS), "{error}");
}

#[cfg(not(feature = "oci-principals"))]
#[test]
fn principal_modes_are_unsupported_without_the_feature() {
    for mode in [
        OciAuthMode::InstancePrincipal,
        OciAuthMode::OkeWorkloadIdentity,
    ] {
        let settings = OciProviderSettings {
            auth_mode: mode,
            region: Some("us-chicago-1".into()),
            compartment_id: Some("ocid1.compartment.test".into()),
            project_id: Some("ocid1.generativeaiproject.test".into()),
            ..Default::default()
        };
        let error = oci_config_from_entry("oci", &settings, "", "m").unwrap_err();
        assert!(error.contains("oci-principals"), "{error}");
        assert!(error.contains(mode.as_str()), "{error}");
    }
}

#[test]
fn stored_key_is_required_only_for_api_key_mode() {
    assert!(entry_requires_stored_key(&oci_entry(api_key_settings())));
    let mut no_block = oci_entry(api_key_settings());
    no_block.oci = None;
    assert!(entry_requires_stored_key(&no_block));
    for mode in [
        OciAuthMode::ConfigProfile,
        OciAuthMode::SessionToken,
        OciAuthMode::ResourcePrincipal,
        OciAuthMode::InstancePrincipal,
        OciAuthMode::OkeWorkloadIdentity,
    ] {
        let mut settings = api_key_settings();
        settings.auth_mode = mode;
        assert!(!entry_requires_stored_key(&oci_entry(settings)), "{mode:?}");
    }
    let bearer = CloudProviderCreds {
        auth_style: AuthStyle::Bearer,
        ..Default::default()
    };
    assert!(entry_requires_stored_key(&bearer));
}

#[test]
fn oci_slug_routes_through_the_cloud_slug_factory() {
    let _guard = crate::inference::inference_test_guard();
    let (_dir, config_file) = oci_dir();
    let mut config = Config::default();
    config.cloud_providers.push(oci_entry(profile_settings(
        OciAuthMode::SessionToken,
        &config_file,
        "SESSION",
    )));
    let (_model, model_id) =
        super::super::cloud_slug::try_create_cloud_slug_chat_model_from_string_with_native_tools(
            "chat",
            "oci:cohere.command-a-03-2025",
            &config,
            true,
            &crate::security::egress::EgressLedger::none(),
        )
        .expect("oci slug is a configured cloud provider")
        .expect("model builds from a session-token profile");
    assert_eq!(model_id, "cohere.command-a-03-2025");
}

#[test]
fn oci_slug_with_a_missing_compartment_fails_through_the_factory() {
    let _guard = crate::inference::inference_test_guard();
    let mut settings = api_key_settings();
    settings.transport = OciTransportKind::NativeGeneric;
    let mut config = Config::default();
    config.cloud_providers.push(oci_entry(settings));
    let error =
        match super::super::cloud_slug::try_create_cloud_slug_chat_model_from_string_with_native_tools(
            "chat",
            "oci:openai.gpt-5.5",
            &config,
            true,
            &crate::security::egress::EgressLedger::none(),
        )
        .expect("oci slug is a configured cloud provider")
        {
            Err(error) => error.to_string(),
            Ok(_) => panic!("a native transport without a compartment must not build"),
        };
    assert!(error.contains("oci.compartment_id"), "{error}");
}

#[test]
fn inspection_records_the_oci_wire_identity() {
    let _guard = crate::inference::inference_test_guard();
    let (_dir, config_file) = oci_dir();
    let mut config = Config::default();
    config.cloud_providers.push(oci_entry(profile_settings(
        OciAuthMode::SessionToken,
        &config_file,
        "SESSION",
    )));
    let mut identity = None;
    super::super::cloud_slug::try_create_cloud_slug_chat_model_from_string_with_native_tools_and_access(
        "chat",
        "oci:cohere.command-a-03-2025",
        &config,
        true,
        &crate::security::egress::EgressLedger::none(),
        None,
        super::super::credentials::CredentialAccess::Inspection,
        Some(&mut identity),
    )
    .expect("oci slug is a configured cloud provider")
    .expect("model builds from a session-token profile");
    let (endpoint, credential, model, _) = identity.expect("the OCI route records its identity");
    // The preset URL on the entry is ignored; the host follows the region.
    assert_eq!(
        endpoint,
        "https://inference.generativeai.us-ashburn-1.oci.oraclecloud.com"
    );
    assert!(
        credential.is_empty(),
        "a session-token profile stores no key"
    );
    assert_eq!(model, "cohere.command-a-03-2025");
}

#[test]
fn home_relative_config_paths_expand_to_the_home_directory() {
    let Some(home) = std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
    else {
        return;
    };
    assert_eq!(expand_home("~"), home);
    assert_eq!(expand_home("~/.oci/config"), home.join(".oci/config"));
    assert_eq!(expand_home("~\\.oci\\config"), home.join(".oci\\config"));
    assert_eq!(
        expand_home("/etc/oci/config"),
        PathBuf::from("/etc/oci/config")
    );
    assert_eq!(expand_home("~user/config"), PathBuf::from("~user/config"));
}
