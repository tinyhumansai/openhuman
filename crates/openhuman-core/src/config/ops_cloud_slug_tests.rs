use super::*;

fn cloud_entry(slug: &str) -> crate::config::schema::cloud_providers::CloudProviderCreds {
    use crate::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};
    CloudProviderCreds {
        id: format!("id-{slug}"),
        slug: slug.into(),
        label: slug.into(),
        endpoint: "https://llm.example.test/v1".into(),
        auth_style: AuthStyle::Bearer,
        default_model: Some("model-1".into()),
    }
}

#[tokio::test]
async fn apply_model_settings_refuses_a_new_cloud_slug_claimed_by_a_local_runtime() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    cfg.api_url = Some("https://before.example.test".into());

    let patch = ModelSettingsPatch {
        // Applied before the provider list in the normal order; a refusal must
        // leave it untouched.
        api_url: Some("https://after.example.test".into()),
        cloud_providers: Some(vec![cloud_entry("vllm")]),
        ..Default::default()
    };
    let err = apply_model_settings(&mut cfg, patch)
        .await
        .expect_err("a local-runtime slug must be refused");

    assert!(err.contains("'vllm'"), "error names the slug: {err}");
    assert!(err.contains("my-vllm"), "error suggests a slug: {err}");
    assert_eq!(cfg.api_url.as_deref(), Some("https://before.example.test"));
    assert!(cfg.cloud_providers.iter().all(|e| e.slug != "vllm"));
}

#[tokio::test]
async fn apply_model_settings_keeps_a_stored_cloud_slug_claimed_by_a_local_runtime() {
    // A config written by an older build may already hold such an entry. The
    // settings panel echoes the whole list on every save, so refusing it would
    // fail every unrelated change.
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    cfg.cloud_providers = vec![cloud_entry("vllm")];

    let patch = ModelSettingsPatch {
        default_model: Some("model-2".into()),
        cloud_providers: Some(vec![cloud_entry("vllm"), cloud_entry("my-vllm")]),
        ..Default::default()
    };
    apply_model_settings(&mut cfg, patch)
        .await
        .expect("the stored entry is tolerated");

    assert_eq!(cfg.default_model.as_deref(), Some("model-2"));
    let slugs: Vec<_> = cfg
        .cloud_providers
        .iter()
        .map(|e| e.slug.as_str())
        .collect();
    assert!(
        slugs.contains(&"vllm") && slugs.contains(&"my-vllm"),
        "{slugs:?}"
    );
}

#[tokio::test]
async fn apply_model_settings_accepts_openai_ollama_and_lmstudio_as_cloud_slugs() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);

    let patch = ModelSettingsPatch {
        cloud_providers: Some(vec![
            cloud_entry("openai"),
            cloud_entry("ollama"),
            cloud_entry("lmstudio"),
        ]),
        ..Default::default()
    };
    apply_model_settings(&mut cfg, patch)
        .await
        .expect("these slugs are not shadowed");
    assert_eq!(
        cfg.cloud_providers
            .iter()
            .filter(|e| ["openai", "ollama", "lmstudio"].contains(&e.slug.as_str()))
            .count(),
        3
    );
}
